# Design

## Context

Projeto novo (repositório vazio). Motivação em `proposal.md`; comportamento em `specs/`. As restrições fixas (Rust, FFmpeg como subprocesso, VMAF médio >= 93 no perfil padrão como definição de "sem perda perceptível") estão em `openspec/config.yaml`. Máquina de referência: FFmpeg 9.0 com libsvtav1, libx265, libx264, NVENC/QSV/VAAPI e libvmaf, e GPU RTX 4060 Ti (NVENC de 8ª geração, com AV1).

## Goals / Non-Goals

**Goals:**
- Um `nago-core` sem dependência de terminal, reutilizável pela futura TUI: recebe uma descrição de job e emite eventos.
- Toda a lógica de decisão (copiar, recodificar ou descartar stream; escolha de encoder; flags) em funções puras, testáveis sem executar o ffmpeg.
- Velocidade: nenhuma recodificação desnecessária, e encoder de hardware a um flag de distância.

**Non-Goals:**
- Reimplementar demux/decodificação ou ter controle frame a frame (exigiria a libav).
- Ajuste fino de qualidade por cena (fica para `add-quality-targeting`).
- Garantir o mesmo desempenho em macOS/Windows nesta change.

## Decisions

### D1. FFmpeg como subprocesso, não bindings da libav
Usar os binários `ffmpeg`/`ffprobe` via `tokio::process`.
- **Por quê:** os encoders são os mesmos; não há compilação contra a libav (cujo ABI muda entre versões maiores, e o FFmpeg 9 já quebra crates de binding); não há vínculo com código GPL (x264/x265), o que permite licença MIT/Apache-2.0; e o usuário pode trocar de FFmpeg sem recompilar.
- **Alternativas:** `ffmpeg-next`/`rsmpeg` (controle total, porém build frágil e manutenção pesada); `gstreamer-rs` (pipeline poderoso, mas dependência pesada e menos ubíqua). A troca continua possível no futuro, pois o core isola a execução atrás de um trait `Executor`.

### D2. Arquitetura em camadas no `nago-core`
```
cli/tui ──> ops (cut, convert, compress)          // monta um Plan a partir de opções + MediaInfo
               │
               ├─> probe     (ffprobe → MediaInfo normalizado)
               ├─> caps      (detecção de encoders/hardware, com cache)
               ├─> planner   (puro: MediaInfo + opções → Plan{streams: Copy|Transcode|Drop, encoder, flags})
               ├─> cmd       (puro: Plan → Vec<OsString> com os argumentos do ffmpeg)
               └─> runner    (executa, faz parse do progresso, cancela, faz rename atômico) ──> Stream<JobEvent>
```
- `JobEvent = Started | Progress{ratio, speed, eta} | Warning(String) | Finished(Report) | Failed(Error)`, entregue por `tokio::sync::mpsc`. A CLI renderiza com `indicatif`; a TUI vai consumir o mesmo canal.
- **Alternativa:** a CLI montar os comandos diretamente. Foi rejeitada porque duplicaria tudo na TUI e impediria testar o planner isoladamente.

### D3. Progresso via `-progress pipe:1 -nostats`
O ffmpeg escreve pares `chave=valor` em stdout. Lemos `out_time_us` e `speed` e calculamos `ratio = out_time / duração_esperada` (a duração do trecho no caso do corte). stderr é guardado num ring buffer (últimas ~50 linhas) para mensagens de erro; o log completo vai para arquivo com `--verbose`.
- **Alternativa:** fazer parse de stderr (`frame= ... time=`), um formato instável e localizável.

### D4. Corte rápido e corte exato
- **Rápido:** `-ss <from>` antes de `-i` (seek por input) + `-t <dur>` + `-c copy -map 0 -avoid_negative_ts make_zero`. Para informar o início real, o core consulta o keyframe anterior com `ffprobe -select_streams v:0 -skip_frame nokey -show_entries frame=pts_time -read_intervals <from-20>%<from+1>`, uma janela pequena para ser instantâneo mesmo em arquivos enormes.
- **Exato (MVP):** recodifica o trecho inteiro com o perfil `visually-lossless` e o mesmo codec da entrada quando houver encoder (senão AV1), copiando o áudio. O seek de entrada vai ao keyframe anterior e um `-ss` de **saída** descarta com precisão o que vem antes do início. Com só o seek de entrada, o áudio copiado carregaria o pré-roll desde o keyframe e ficaria dessincronizado (constatado nos testes).
- **Futuro (`add-smart-cut`):** recodificar só o GOP inicial e o final e concatenar o miolo por stream copy. É muito mais rápido, mas exige parâmetros de encoder compatíveis com o stream original, então fica fora do MVP.

### D5. Matriz de compatibilidade container × codec
Uma tabela estática (`planner::compat`) define, para cada container, os codecs aceitos por tipo de stream e o codec de fallback:

| Container | Vídeo aceito | Áudio aceito (fallback) | Legenda aceita |
|-----------|--------------|-------------------------|----------------|
| mp4/mov   | h264, hevc, av1 (+ prores, só em mov) | aac, mp3, opus*, ac3, eac3, alac, flac* (→ aac) | mov_text (srt/ass → mov_text; imagem → descarta) |
| mkv       | qualquer     | qualquer                | qualquer       |
| webm      | av1, vp9 (→ av1) | opus, vorbis (→ opus) | webvtt (srt → webvtt; demais → descarta) |

\* opus e flac em mp4 são válidos, mas têm suporte irregular em players; o fallback é aac quando `--compat` for informado (padrão: copiar).
A decisão por stream (`Copy | Transcode(codec) | Drop(motivo)`) é uma função pura coberta por testes de tabela.

### D6. Perfis de qualidade (calibrados na tarefa 6.6, ver `docs/calibration.md`)

| Perfil | Meta VMAF | SVT-AV1 (preset 6) | x265 (slow) | x264 (slow) | NVENC AV1 | NVENC HEVC | NVENC H.264 |
|--------|-----------|--------------------|-------------|-------------|-----------|------------|-------------|
| visually-lossless | 95 | crf 15 | crf 17 | crf 15 | cq 27 | cq 21 | cq 23 |
| high (padrão)     | 93 | crf 24 | crf 21 | crf 19 | cq 32 | cq 26 | cq 27 |
| balanced          | 91 | crf 30 | crf 24 | crf 21 | cq 36 | cq 28 | cq 29 |
| small             | 88 | crf 35 | crf 27 | crf 24 | cq 40 | cq 31 | cq 32 |

- Qualidade constante (CRF/CQ), nunca bitrate fixo: o tamanho se adapta ao conteúdo, e qualidade é o que o usuário pediu.
- **Saída 10-bit** para AV1/HEVC (`yuv420p10le`, ou `p010le` no NVENC) mesmo com fonte 8-bit: comprime melhor e evita banding. H.264 fica em 8-bit por compatibilidade.
- SVT-AV1 com `-svtav1-params tune=0` (otimizado para qualidade visual) e GOP de ~10 s (`-g` = 10×fps) para facilitar o seek.
- NVENC: `-rc vbr -cq N -b:v 0 -multipass qres -spatial-aq 1 -temporal-aq 1 -rc-lookahead 32`.
- Critério de calibração: o maior CRF/CQ cujo VMAF médio atinge a meta no **pior** dos 4 clipes do corpus (sempre a gravação de celular já comprimida). Conteúdo limpo (filmes, animação) fica bem acima da meta. A tabela fica num único módulo (`profiles.rs`); QSV/VAAPI reutilizam os valores do NVENC sem calibração própria.

### D7. Detecção de encoders com teste real
`ffmpeg -encoders` só diz o que foi compilado. Para cada encoder de hardware candidato, o core executa um teste de ~0,1 s (`-f lavfi -i color=s=256x256:d=0.1 -c:v <enc> -f null -`). O resultado vai para cache em `$XDG_CACHE_HOME/nago/caps.json`, com chave (caminho do ffmpeg, mtime, versão). `nago doctor` sempre refaz a detecção.
- **Alternativa:** checar dispositivos (`/dev/dri`, `nvidia-smi`). É frágil e não prova que o ffmpeg consegue usá-los.

### D8. HDR
Aplicar `color_primaries`, `color_trc`, `colorspace` e `color_range` do probe via filtro `setparams` (no FFmpeg 9 as flags de saída `-color_*` não chegam ao container MKV ao codificar; `setparams` marca os frames e funciona em todos os containers). Mastering display e MaxCLL são lidos de `side_data` do primeiro frame (`ffprobe -show_frames -read_intervals %+#1`) e repassados via `-svtav1-params mastering-display=...:content-light=...` ou `-x265-params master-display=...:max-cll=...`. O NVENC usa os metadados do stream. HDR + H.264 é recusado (spec `video-compression`).

### D9. Escrita atômica e cancelamento
- Saída temporária: `<dir_destino>/.<nome>.nago-<rand>.part`, com `-f <muxer>` explícito (a extensão não é confiável). Um guard RAII remove o temporário no `Drop`; em caso de sucesso, `std::fs::rename` (atômico no mesmo filesystem).
- Sinais via `tokio::signal`. Ao cancelar, o core envia `q` no stdin do ffmpeg (encerramento gracioso), depois SIGTERM após 2 s e SIGKILL após 5 s. O guard limpa e a CLI sai com 130.

### D10. VMAF (`--verify`)
`ffmpeg -i <saída> -i <entrada> -lavfi "[0:v]format=yuv420p10le,settb=AVTB,setpts=N[d];[1:v]…[r];[d][r]libvmaf=model=version=vmaf_v0.6.1:n_threads=<cpus>:log_fmt=json:log_path=<tmp>" -f null -`. Os frames são pareados **por índice** (`setpts=N`): pareando por timestamp, o arredondamento para milissegundos do MKV comparava frames vizinhos e derrubava o VMAF de ~95 para ~84, com p5 em torno de 8 (constatado na calibração). A média e o p5 são calculados a partir do log por frame. Para HDR, o sistema avisa que o modelo VMAF padrão é SDR e que o valor é apenas indicativo.

### D11. Crates e ferramentas
`clap` (derive), `tokio` (rt-multi-thread, process, signal, sync), `serde`/`serde_json`, `thiserror`/`anyhow`, `tracing` + `tracing-subscriber`, `indicatif`, `tempfile`, `humansize`, `which`. Testes: `insta` (snapshots dos argumentos do ffmpeg), `assert_cmd` + `predicates` (CLI). CI: GitHub Actions com `cargo fmt --check`, `clippy -D warnings` e testes (Ubuntu 24.04, ffmpeg 6.1 via apt). Testes de GPU ficam marcados com `#[ignore]` e rodam localmente com `cargo test -- --ignored`.

## Risks / Trade-offs

- [O VMAF satura em ~97,5 mesmo para vídeo idêntico (vmaf_v0.6.1, pouco movimento) e fontes já comprimidas exigem CRF muito baixo para passar de 95] → metas fixadas em 95/93/91/88 (≈ 1 JND abaixo do teto no `high`), após calibração com fontes reais; números em `docs/calibration.md`.
- [CRF fixo não garante a meta de VMAF em todo conteúdo (anime, grão, tela)] → `--verify` mede e avisa; a busca automática de CRF vem em `add-quality-targeting`.
- [NVENC gera arquivos ~15–30% maiores que o SVT-AV1 para a mesma qualidade] → software é o padrão; `--gpu` é uma escolha consciente por velocidade, e o relatório mostra o encoder usado.
- [O corte rápido começa no keyframe anterior, então pode incluir até um GOP a mais] → o sistema informa o início real e sugere `--exact`.
- [O corte exato recodifica o trecho inteiro e é lento para trechos longos] → é aceitável no MVP; o smart cut fica para uma change própria.
- [Dolby Vision: a camada RPU é perdida na recodificação] → detectar no probe e avisar que a saída será HDR10 (base layer); suporte a DV fica fora do escopo.
- [Fontes com frame rate variável (gravações de celular/tela)] → usar `-fps_mode passthrough` para não duplicar ou descartar frames.
- [Diferenças de opções entre versões do ffmpeg] → mínimo 6.0, `cmd` centraliza todas as flags e o CI testa com 6.1 enquanto o desenvolvimento local usa 9.0.
- [rename entre filesystems diferentes não é atômico] → o temporário é sempre criado no diretório de destino.

## Migration Plan

Não se aplica (projeto novo). Distribuição inicial: `cargo install --path crates/nago-cli`; pacote AUR e binários de release ficam para depois do MVP.

## Roadmap das próximas changes

1. `add-tui`: modo interativo (`nago` sem argumentos) com ratatui, seleção de arquivo, preview de metadados, fila e progresso, consumindo `JobEvent`.
2. `add-quality-targeting`: `--target-vmaf N`, com busca binária de CRF em amostras curtas do vídeo, no estilo do ab-av1.
3. `add-batch-queue`: múltiplos arquivos ou pastas, jobs paralelos limitados por CPU/GPU, retomada.
4. `add-smart-cut`: corte exato recodificando apenas as bordas de GOP.
5. `add-user-config`: `~/.config/nago/config.toml` com perfis personalizados e padrões.

## Open Questions

- Licença final (proposta: MIT OR Apache-2.0). Não afeta specs nem tarefas.
- Valores finais da tabela D6, definidos pela calibração; mudam constantes, não a estrutura.
