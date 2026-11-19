# Proposal

## Why

Cortar, converter e comprimir vídeo hoje exige decorar flags do FFmpeg (CRF, preset, pix_fmt, keyframes, aceleração por hardware), e errar qualquer uma delas resulta em arquivo maior, perda visível de qualidade ou corte impreciso. O `nago` encapsula essas decisões em comandos simples com padrões seguros, entregando arquivos menores e visualmente idênticos ao original. Esta é a change fundadora: cria o core em Rust e a CLI sobre a qual a TUI e os recursos avançados serão construídos.

## What Changes

- Novo workspace Rust com `nago-core` (biblioteca) e `nago-cli` (binário `nago`).
- `nago info <arquivo>`: inspeciona o arquivo via `ffprobe` (container, codecs, duração, bitrate, resolução, HDR, faixas).
- `nago cut <arquivo> --from <t> --to <t>`: corte instantâneo sem recodificação (alinhado a keyframe) por padrão, e corte preciso ao frame com `--exact`.
- `nago convert <arquivo> --to <formato>`: troca de container com stream copy (remux) quando os codecs são compatíveis, e recodificação somente quando necessário.
- `nago compress <arquivo>`: recodificação com perfis de qualidade (`--quality visually-lossless|high|balanced|small`) e codec (`--codec av1|hevc|h264`), com AV1 como padrão.
- Seleção automática de encoder: detecta NVENC/QSV/VAAPI e usa o encoder de software (SVT-AV1/x265/x264) como fallback ou quando `--cpu` é pedido.
- Execução de jobs: barra de progresso (via `-progress` do ffmpeg), cancelamento com Ctrl+C sem deixar lixo, escrita atômica da saída, saída `--json` para scripts.
- Verificação opcional de qualidade com `--verify`, que mede o VMAF entre a saída e o original e informa o resultado.
- Proteção contra resultado pior: se a saída comprimida ficar maior que a entrada, o `nago` avisa e mantém o original (a menos que `--force`).

## Capabilities

### New Capabilities
- `media-probe`: leitura e normalização dos metadados de mídia (streams, duração, codecs, HDR, keyframes).
- `video-cut`: extração de trechos por intervalo de tempo, nos modos rápido (stream copy) e exato.
- `container-conversion`: troca de container/formato preservando streams sempre que possível.
- `video-compression`: recodificação orientada a qualidade com perfis, codecs e verificação VMAF opcional.
- `encoder-selection`: detecção de capacidades do FFmpeg/hardware e escolha do encoder.
- `job-execution`: execução de processos do ffmpeg com progresso, cancelamento, escrita atômica e relatório.

### Modified Capabilities
- Nenhuma (projeto novo).

## Fora do escopo

- TUI interativa (change futura `add-tui`, sobre o mesmo `nago-core`).
- Busca automática de CRF por meta de VMAF (`--target-vmaf`); nesta change o VMAF só é medido (change futura `add-quality-targeting`).
- Processamento em lote, pastas e filas paralelas (change futura `add-batch-queue`).
- Interface gráfica, edição com múltiplos cortes, concatenação, filtros (crop, scale, legendas queimadas).
- Distribuir o FFmpeg embutido; ele é uma dependência de runtime instalada pelo usuário.
- Codificação matematicamente lossless como modo principal.

## Impact

- Código: repositório novo (`Cargo.toml` workspace, `crates/nago-core`, `crates/nago-cli`).
- Dependências Rust: tokio, clap, serde/serde_json, thiserror, anyhow, tracing, indicatif, tempfile.
- Runtime: exige `ffmpeg` e `ffprobe` >= 6.0 no PATH; `libvmaf` é necessária apenas para `--verify`.
- Sistema: nenhum serviço ou configuração global; nada é gravado fora do diretório de saída e do diretório temporário.
