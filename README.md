# nago

Corte, converta e comprima vídeos de forma rápida e simples, deixando o arquivo
menor **sem perda de qualidade perceptível**.

O `nago` é uma CLI (com modo interativo no terminal) em Rust que usa o FFmpeg instalado na máquina e toma por você as
decisões chatas: CRF, preset, pixel format, keyframes, HDR, compatibilidade de
container e aceleração por GPU.

```console
$ nago compress aula.mp4
✓ aula.compressed.mp4
  1.20 GB → 380 MB (-68%) em 4m 12s
  encoder: libsvtav1 · perfil high
  streams: #0 vídeo h264 → libsvtav1 · #1 áudio aac copiado
```

## Instalação

Requisitos: FFmpeg ≥ 6.0 (com `libsvtav1`, `libx265` e `libx264`; `libvmaf` é
necessária apenas para `--verify`) e Rust ≥ 1.85.

```sh
cargo install --path crates/nago-cli
nago doctor    # confere o ffmpeg, os encoders e se a GPU funciona
```

Para usar outro ffmpeg, aponte `NAGO_FFMPEG` e `NAGO_FFPROBE` para os binários.

## Comandos

| Comando | O que faz |
|---|---|
| `nago info <arquivo>` | Mostra container, duração, streams, HDR (`--json` para scripts) |
| `nago cut <arquivo>` | Extrai um trecho (instantâneo ou exato ao frame) |
| `nago convert <arquivo>` | Troca o container copiando os streams sempre que possível |
| `nago compress <arquivo>` | Recodifica para reduzir o tamanho mantendo a qualidade |
| `nago doctor` | Diagnóstico do ffmpeg, encoders e aceleração por hardware |

Opções globais: `--json` (relatório em JSON em stdout), `-v/--verbose` (salva o
log completo do ffmpeg), `--overwrite` (permite sobrescrever o arquivo de `-o`;
a entrada nunca é sobrescrita).

### `nago compress`

```sh
nago compress video.mp4                       # AV1, perfil high → video.compressed.mp4
nago compress video.mp4 --quality visually-lossless
nago compress video.mp4 --codec hevc          # mais compatível com TVs e celulares antigos
nago compress video.mp4 --gpu                 # NVENC/QSV/VAAPI: muito mais rápido
nago compress video.mp4 --verify              # mede o VMAF ao final
nago compress video.mkv --audio-bitrate 128   # também recodifica o áudio (Opus)
nago compress video.mp4 -o menor.mkv
```

**Perfis de qualidade** (`--quality`), com a meta de VMAF médio de cada um:

| Perfil | Meta VMAF | Quando usar |
|---|---|---|
| `visually-lossless` | ≥ 95 | Arquivo/master: indistinguível do original mesmo procurando |
| `high` (padrão) | ≥ 93 | Uso geral: sem diferença visível assistindo normalmente |
| `balanced` | ≥ 91 | Compartilhar: diferença só em comparação lado a lado |
| `small` | ≥ 88 | Prioridade é o tamanho (pré-visualizações, envio rápido) |

**Codec** (`--codec`): `av1` (padrão, o menor arquivo), `hevc` (bom equilíbrio e
suporte amplo em hardware) ou `h264` (compatibilidade máxima, arquivo maior).
AV1 e HEVC saem em 10-bit, o que comprime melhor e evita banding.

**GPU** (`--gpu`): usa o encoder de hardware na ordem NVENC > QSV > VAAPI. É mais
rápido, mas o arquivo sai bem maior para a mesma qualidade: na calibração, o NVENC AV1
no perfil `high` reduziu de 10% a 23% as gravações de celular, contra 39% a 41% do
SVT-AV1. Sem GPU funcional, o `nago` avisa e usa o encoder de software.

O que o `nago compress` garante:

- resolução, fps e orientação iguais aos da entrada; áudio e legendas copiados
  sem recodificar;
- **HDR preservado** (HDR10/HLG, mastering display e MaxCLL). HDR com `--codec h264`
  é recusado em vez de gerar cores erradas;
- se o resultado ficar **maior** que o original, a saída é descartada e o comando sai
  com código 3 (use `--force` para manter mesmo assim);
- `--verify` calcula o VMAF médio e o percentil 5 e avisa se a meta do perfil não
  foi atingida.

Container de saída: o mesmo da entrada quando ele suporta o codec; senão, `mkv`.

Os valores de CRF/CQ de cada perfil foram calibrados com VMAF em 4 clipes reais; os
resultados por encoder estão em [`docs/calibration.md`](docs/calibration.md).

### `nago cut`

```sh
nago cut aula.mp4 --from 1:30 --to 2:45.5      # → aula.cut.mp4, instantâneo
nago cut aula.mp4 --from 90 --duration 30       # 30 s a partir de 1:30
nago cut aula.mp4 --from 1:30 --to 1:40 --exact # preciso ao frame
nago cut aula.mp4 --to 10 -o inicio.mp4         # do começo até 0:10
```

Os instantes aceitam `SS[.ms]`, `MM:SS[.ms]` ou `HH:MM:SS[.ms]`.

| Modo | Velocidade | Precisão | Qualidade |
|---|---|---|---|
| rápido (padrão) | instantâneo, sem recodificar | começa no **keyframe** anterior ao `--from` (o `nago` informa o início real) | idêntica ao original |
| `--exact` | recodifica o trecho | ±1 frame no início e no fim | perfil `visually-lossless` |

Use o modo rápido para cortes "por cima" (tirar o começo de uma gravação,
separar partes de uma aula) e `--exact` quando o frame exato importa. Todas as
faixas de áudio e legenda são mantidas, com seus idiomas.

### `nago convert`

```sh
nago convert video.mkv --to mp4        # remux: instantâneo, sem perda
nago convert video.mkv -o video.mp4    # formato inferido pela extensão
nago convert video.mp4 --to webm       # recodifica para AV1 + Opus (exigência do WebM)
nago convert video.mkv --to m4a        # extrai o áudio
nago convert video.mkv --to mp3 --audio-track 2
nago convert video.mkv --to mp4 --compat   # Opus/FLAC → AAC para players antigos
```

Formatos: `mp4`, `mkv`, `mov`, `webm` (vídeo) e `m4a`, `mp3`, `opus`, `flac` (áudio).

O `convert` **copia** todo stream que cabe no formato de destino e recodifica só
o que é incompatível, informando o quê e por quê. Exemplos: áudio PCM vira AAC em
MP4; legendas SRT viram `mov_text` (MP4) ou WebVTT (WebM); legendas de imagem (PGS)
que o destino não suporta são descartadas com aviso. Saídas MP4/MOV/M4A já saem
otimizadas para streaming (índice no início do arquivo).

## Modo interativo (TUI)

Rodar `nago` sem argumentos num terminal abre o modo interativo. Também dá para abrir
com arquivos já selecionados: `nago tui video1.mp4 video2.mkv`. Em scripts e pipes
(sem terminal), `nago` sem subcomando continua mostrando a ajuda e saindo com código 2.

O fluxo é: escolher arquivos no navegador, ver os metadados, abrir uma operação, conferir
o resultado previsto e enfileirar. A fila roda um job por vez enquanto você continua
navegando e enfileirando.

- **Navegador**: lista pastas e arquivos de mídia da pasta atual (`.` mostra todos) e
  mostra ao lado o mesmo conteúdo do `nago info`. `Espaço` marca vários arquivos: a
  operação gera um job por arquivo.
- **Formulários**: as mesmas opções e padrões da CLI. Antes de enfileirar, aparecem o
  caminho de saída, o encoder (software ou GPU) e os streams. Erros aparecem no próprio
  campo, com a mesma mensagem da CLI.
- **Corte**: início e fim ajustáveis por frame, por segundo ou de keyframe em keyframe.
  O painel avisa onde o corte rápido vai começar de fato (o keyframe anterior) e
  sugere o modo exato quando a diferença passa de 1 s.
- **Miniatura**: em terminais com protocolo de imagem (kitty, Ghostty, WezTerm, iTerm2
  ou terminais com sixel), o corte mostra o frame do início ou do fim. Em terminais sem
  suporte, só os tempos são exibidos. `NAGO_TUI_IMAGES=off|kitty|sixel|iterm2` desliga
  a miniatura ou força um protocolo (útil dentro do tmux, onde a detecção pode falhar).
- **Fila** (`Tab`): estado, fase, progresso, velocidade e tempo restante de cada job e,
  ao terminar, o mesmo relatório da CLI (incluindo o VMAF). Jobs que falham mostram o
  erro e o fim do log do ffmpeg, e a fila segue para o próximo.

Sair com jobs ativos pede confirmação. O job em execução é cancelado sem deixar arquivo
parcial, os pendentes são descartados e o código de saída é 130. Sem jobs interrompidos
o código é 0, mesmo que algum job tenha falhado. Com `-v`, os logs vão para
`$XDG_STATE_HOME/nago/tui.log`.

**Navegador**

| Teclas | Ação |
|---|---|
| `↑/↓` `k/j` | navegar |
| `Enter` `l` | entrar na pasta |
| `Backspace` `h` | pasta pai |
| `Espaço` | marcar/desmarcar arquivo |
| `.` | mostrar todos os arquivos / só mídia |
| `c` | cortar |
| `v` | converter |
| `z` | comprimir |
| `Tab` | ver a fila |
| `q` | sair |

**Formulários (converter, comprimir)**

| Teclas | Ação |
|---|---|
| `↑/↓` `Tab/Shift+Tab` | campo anterior/seguinte |
| `←/→` `Espaço` | mudar opção / alternar |
| `Backspace` `Ctrl+U` | apagar caractere / limpar campo |
| `Enter` | enfileirar |
| `Esc` | voltar ao navegador |

**Corte**

| Teclas | Ação |
|---|---|
| `i / o` | editar início / fim |
| `< / >` | 1 frame para trás / para frente |
| `←/→` | 1 s para trás / para frente |
| `Shift+←/→` | 10 s para trás / para frente |
| `[ / ]` | keyframe anterior / seguinte |
| `Enter` | enfileirar |
| `Esc` | voltar ao navegador |

**Fila**

| Teclas | Ação |
|---|---|
| `↑/↓` `k/j` | selecionar job |
| `x` | cancelar o job em execução |
| `d` | remover o job selecionado (pendente ou terminado) |
| `Tab` `Esc` | voltar ao navegador |
| `q` | sair |

**Em qualquer tela**

| Teclas | Ação |
|---|---|
| `?` | mostrar/ocultar esta ajuda |
| `Ctrl+C` | sair (duas vezes seguidas confirma) |

## Desempenho

Fluxo ponta a ponta numa gravação de celular de 1 minuto (H.264 1080x1920, 30 fps,
21,36 MB), com Ryzen de 16 threads e RTX 4060 Ti, FFmpeg 9:

| Comando | Tempo | Resultado |
|---|---|---|
| `nago cut video.mp4 --from 10 --to 25` | 0,2 s | 5,65 MB (stream copy) |
| `nago cut ... --exact` | 6,7 s | 10,19 MB (recodifica, `visually-lossless`) |
| `nago convert video.mp4 --to mkv` | 0,2 s | 21,35 MB (stream copy) |
| `nago compress video.mp4 --verify` | 1 min 00 s | 12,50 MB (−41%), VMAF 94,49 · p5 93,22 |
| `nago compress video.mp4 --gpu --verify` | 38 s | 19,11 MB (−11%), VMAF 94,74 · p5 93,50 |

Fontes já comprimidas (celular, web) são o pior caso. Filmes e animação com mais
bitrate reduzem de 50% a 90% no perfil `high` (ver a calibração).

## Códigos de saída

| Código | Significado |
|---|---|
| 0 | sucesso |
| 1 | erro de execução (ffmpeg ausente, falha na codificação, encoder indisponível) |
| 2 | erro de uso ou validação (opção inválida, intervalo inválido, conflito de saída) |
| 3 | compressão descartada porque não reduziria o tamanho |
| 130 | cancelado (Ctrl+C); nenhum arquivo parcial permanece |

## Desenvolvimento

```sh
cargo test                       # testes (gera fixtures sintéticas com ffmpeg)
cargo test -- --include-ignored  # inclui testes que exigem GPU NVIDIA
cargo lint                       # clippy com -D warnings
```

O projeto é planejado com [OpenSpec](https://github.com/Fission-AI/OpenSpec): as
specs ficam em `openspec/` e as decisões de arquitetura em
`openspec/changes/archive/*/design.md` (changes concluídas).

## Licença

MIT ou Apache-2.0, à sua escolha.
