# Tasks

## 1. Estrutura do workspace

- [x] 1.1 Criar o workspace Cargo (edition 2024) com `crates/nago-core` (lib) e `crates/nago-cli` (bin `nago`), `rust-toolchain.toml`, `.gitignore` e licença MIT OR Apache-2.0; verificar que `cargo build` compila e `cargo run -p nago-cli -- --help` mostra a ajuda
- [x] 1.2 Adicionar as dependências de D11 e configurar `clippy` (`-D warnings`) e `rustfmt`; verificar que `cargo clippy --all-targets` e `cargo fmt --check` passam
- [x] 1.3 Criar o esqueleto da CLI com clap: subcomandos `info`, `cut`, `convert`, `compress`, `doctor`, flags globais `--json`, `--verbose`, `--overwrite` e `-o`, e o mapeamento de erros para os códigos 0/1/2/3/130; verificar com um teste `assert_cmd` em que opção desconhecida resulta em código 2
- [x] 1.4 Criar o helper de testes que gera fixtures sintéticas com `ffmpeg -f lavfi` (testsrc2 + sine, H.264/AAC 1080p 10 s; HEVC 10-bit HDR10; MOV com PCM; MKV com 2 áudios + legenda SRT) em diretório temporário; verificar que um teste de fumaça gera e lê cada fixture

## 2. Localização do FFmpeg e utilitários base

- [x] 2.1 Implementar a localização de `ffmpeg`/`ffprobe` (`NAGO_FFMPEG`/`NAGO_FFPROBE` > PATH) e o parse de versão, exigindo >= 6.0; verificar com testes unitários de parse (`n9.0.2`, `6.1.1-3ubuntu5`, `5.1`, builds git `N-xxxxx`) e um teste de CLI com PATH vazio que retorna código 1 e a dica de instalação
- [x] 2.2 Implementar o parse e a formatação de tempo (`SS[.ms]`, `MM:SS[.ms]`, `HH:MM:SS[.ms]`) com erros descritivos; verificar com testes de tabela, incluindo casos inválidos e negativos
- [x] 2.3 Implementar a resolução do caminho de saída (nome padrão `<nome>.<sufixo>.<ext>`, sufixo `-N` em caso de colisão, recusa de saída igual à entrada mesmo via symlink, conflito com `-o` sem `--overwrite`); verificar com testes unitários de cada cenário da spec `job-execution` › Proteção de arquivos existentes

## 3. media-probe

- [x] 3.1 Implementar `probe` (ffprobe JSON → `MediaInfo` normalizado: container, duração, tamanho, bitrate, streams com codec, idioma, disposição, resolução, fps, bit depth, cor, canais); verificar com testes contra as fixtures do item 1.4
- [x] 3.2 Detectar HDR10, HLG e Dolby Vision (este último só com aviso) e ler mastering display/MaxCLL do primeiro frame; verificar com a fixture HDR10 (`hdr: "HDR10"`) e a SDR (`hdr: null`)
- [x] 3.3 Implementar a consulta do keyframe anterior a um instante (janela `-read_intervals`); verificar que, em uma fixture com GOP de 2 s, o instante 5,3 s retorna 4,0 s
- [x] 3.4 Implementar `nago info` (tabela humana e `--json` com esquema estável) e os erros de arquivo inexistente e não-mídia; verificar com testes `assert_cmd` cobrindo todos os cenários da spec `media-probe` e um snapshot `insta` do JSON

## 4. encoder-selection

- [x] 4.1 Fazer parse de `ffmpeg -encoders` e da presença do filtro `libvmaf`; verificar com teste unitário sobre uma saída gravada do FFmpeg 9
- [x] 4.2 Implementar o teste funcional de encoders de hardware (D7) com timeout e captura do motivo da falha; verificar que um encoder inexistente é marcado indisponível com motivo e rodar localmente o teste `#[ignore]` que confirma `av1_nvenc` funcional na RTX 4060 Ti
- [x] 4.3 Implementar o cache de capacidades em `$XDG_CACHE_HOME/nago/caps.json`, invalidado por caminho, mtime e versão; verificar com teste que trocar o mtime força nova detecção
- [x] 4.4 Implementar a política de seleção (software por padrão; `--gpu` → NVENC > QSV > VAAPI com fallback e aviso; erro com alternativas quando não há encoder); verificar com testes de tabela usando capacidades simuladas
- [x] 4.5 Implementar `nago doctor` (versão, encoders por codec com status e motivo, libvmaf); verificar com teste `assert_cmd` e conferir manualmente a saída nesta máquina

## 5. job-execution (runner)

- [x] 5.1 Implementar o `Runner`: spawn do ffmpeg com `-progress pipe:1 -nostats -y`, parse de `out_time_us`/`speed`, emissão de `JobEvent` por mpsc e ring buffer de stderr; verificar com teste de parse sobre uma captura real de `-progress` e um teste de integração que recebe eventos `Progress` crescentes e `Finished`
- [x] 5.2 Implementar a saída temporária com guard RAII e o rename atômico (D9); verificar com teste em que o ffmpeg falha (argumento inválido) e não sobra arquivo final nem `.part`
- [x] 5.3 Implementar o cancelamento (SIGINT/SIGTERM → `q` → SIGTERM → SIGKILL) e a limpeza; verificar com teste de integração que envia SIGINT ao processo `nago` durante uma compressão longa e confere o código 130 e a ausência de arquivos residuais
- [x] 5.4 Implementar a renderização na CLI: barra `indicatif` só quando stderr é TTY, relatório humano (`1.2 GB → 380 MB (-68%)`), relatório `--json` exclusivo em stdout, erro resumido com as últimas linhas do log e log completo com `--verbose`; verificar com testes `assert_cmd` (sem TTY não há barra; `--json` gera JSON válido com os campos da spec)

## 6. video-compression

- [x] 6.1 Implementar `profiles.rs` com a tabela D6 (perfil × encoder → flags) e os pixel formats 10-bit/8-bit; verificar com snapshots `insta` dos argumentos para cada combinação perfil × encoder
- [x] 6.2 Implementar o planner de compressão: vídeo recodificado mantendo resolução, fps (`-fps_mode passthrough`) e rotação; áudio e legendas copiados; `--audio-bitrate` → Opus/AAC; escolha do container de saída; verificar com testes unitários do planner, incluindo a entrada AVI → `.compressed.mkv`
- [x] 6.3 Implementar o repasse de HDR (cores + mastering display/MaxCLL via svtav1/x265 params) e a recusa de HDR com H.264; verificar com teste de integração sobre a fixture HDR10 (a saída AV1 tem `smpte2084`/`bt2020` no probe) e um teste de CLI para o caso H.264 (código 2)
- [x] 6.4 Implementar a proteção contra saída maior que a entrada (descarte, aviso, código 3, `--force`); verificar com teste que comprime uma fixture AV1 de bitrate muito baixo com `--quality visually-lossless`
- [x] 6.5 Implementar `--verify` (D10: pré-checagem da libvmaf, cálculo de média e p5 a partir do log JSON, aviso abaixo da meta, aviso de HDR); verificar com teste de integração que o relatório contém `vmaf_mean`/`vmaf_p5` e com teste que simula a ausência de libvmaf (código 1 antes de codificar)
- [ ] 6.6 Calibrar a tabela D6: rodar `nago compress --verify` em cada perfil sobre 3 a 4 clipes reais variados (live action, animação, gravação de tela, com grão), ajustar os CRF/CQ para atingir as metas e registrar os resultados em `docs/calibration.md`; verificar que o perfil `high` atinge VMAF médio >= 95 em todos os clipes
- [x] 6.7 Documentar `nago compress` no README (perfis, codecs, `--gpu`, `--verify`, tabela de quando usar cada um); verificar que os exemplos do README rodam como escritos

## 7. container-conversion

- [x] 7.1 Implementar a matriz de compatibilidade D5 e a decisão por stream (`Copy | Transcode | Drop`); verificar com testes de tabela cobrindo todas as linhas da matriz
- [x] 7.2 Implementar `nago convert` (inferência por `-o`, formatos de vídeo, `+faststart` em mp4/mov/m4a, relatório de streams recodificados/descartados); verificar com testes de integração: MKV H.264/AAC → MP4 com streams idênticos (hash dos pacotes via `ffmpeg -f streamhash`), MOV PCM → MP4 com AAC, MP4 → WebM com AV1/Opus e `moov` antes de `mdat`
- [x] 7.3 Implementar a conversão de legendas (srt/ass → mov_text/webvtt; imagem descartada com aviso por faixa) e a extração de áudio (`m4a`/`mp3`/`opus`/`flac`, `--audio-track`); verificar com testes de integração sobre a fixture MKV com legenda e um teste de extração AAC → m4a sem recodificação
- [x] 7.4 Documentar `nago convert` no README; verificar que os exemplos rodam

## 8. video-cut

- [x] 8.1 Implementar a validação do intervalo (`--from`/`--to`/`--duration`, limites, aviso de fim além da duração); verificar com testes `assert_cmd` para cada cenário de erro da spec (código 2, nenhum arquivo criado)
- [x] 8.2 Implementar o corte rápido (D4: seek por input, `-c copy -map 0`, `avoid_negative_ts`, aviso do início real com sugestão de `--exact`); verificar com teste de integração em que todas as faixas são preservadas e o início reportado é igual ao keyframe esperado
- [x] 8.3 Implementar o corte exato (recodificação do trecho com perfil `visually-lossless`, áudio sincronizado); verificar com teste em que a duração da saída de um corte de 10 s a 30 fps difere em no máximo 33 ms e o primeiro frame bate com o original (comparação de frame via `-f framemd5` ou SSIM >= 0,99)
- [x] 8.4 Documentar `nago cut` no README (diferença entre rápido e exato); verificar que os exemplos rodam

## 9. Integração final

- [ ] 9.1 Configurar o GitHub Actions (fmt, clippy, testes em Ubuntu 24.04 com ffmpeg 6.1); verificar que o pipeline passa em um push de teste
- [ ] 9.2 Rodar o fluxo ponta a ponta com um vídeo real nesta máquina (`info` → `cut` → `convert` → `compress --verify` → `compress --gpu`) e registrar os tempos e as reduções de tamanho no README; verificar que todos os comandos terminam com código 0 e que a suíte `cargo test -- --include-ignored` passa localmente
- [x] 9.3 Rodar `openspec validate add-core-cli --strict` e conferir cada cenário das specs contra os testes existentes; verificar que todo cenário tem ao menos um teste correspondente

## Workflow follow-up

- Arquivar a change com `/openspec-archive-change add-core-cli` após a revisão, promovendo as specs para `openspec/specs/`.
- Iniciar a próxima change do roadmap (`add-tui` ou `add-quality-targeting`) com `/openspec-propose`.
