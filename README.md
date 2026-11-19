# nago

Corte, converta e comprima vídeos de forma rápida e simples, deixando o arquivo
menor **sem perda de qualidade perceptível**.

O `nago` é uma CLI em Rust que usa o FFmpeg instalado na máquina e toma por você as
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
| `visually-lossless` | ≥ 97 | Arquivo/master: indistinguível do original mesmo procurando |
| `high` (padrão) | ≥ 95 | Uso geral: sem diferença visível assistindo normalmente |
| `balanced` | ≥ 93 | Compartilhar: diferença só em comparação lado a lado |
| `small` | ≥ 90 | Prioridade é o tamanho (pré-visualizações, envio rápido) |

**Codec** (`--codec`): `av1` (padrão, o menor arquivo), `hevc` (bom equilíbrio e
suporte amplo em hardware) ou `h264` (compatibilidade máxima, arquivo maior).
AV1 e HEVC saem em 10-bit, o que comprime melhor e evita banding.

**GPU** (`--gpu`): usa o encoder de hardware na ordem NVENC > QSV > VAAPI. Fica de 5
a 10 vezes mais rápido, mas o arquivo sai de 15% a 30% maior para a mesma qualidade.
Sem GPU funcional, o `nago` avisa e usa o encoder de software.

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
`openspec/changes/*/design.md`.

## Licença

MIT ou Apache-2.0, à sua escolha.
