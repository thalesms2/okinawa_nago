# Spec Delta

## Purpose

Ler e normalizar os metadados de um arquivo de mídia para que o usuário entenda o que tem em mãos e para que os demais comandos tomem decisões (copiar ou recodificar, preservar HDR, alinhar cortes).

## ADDED Requirements

### Requirement: Exibir metadados do arquivo
O sistema SHALL fornecer o comando `nago info <arquivo>`, que exibe container, duração, tamanho, bitrate total e, para cada stream, tipo, codec, idioma e parâmetros principais (resolução, fps, profundidade de bits e espaço de cor para vídeo; canais e sample rate para áudio).

#### Scenario: Arquivo de vídeo válido
- **WHEN** o usuário executa `nago info filme.mkv` em um arquivo com 1 stream de vídeo H.264 1080p, 2 de áudio e 1 de legenda
- **THEN** o sistema exibe os 4 streams com seus codecs e parâmetros, e a duração e o tamanho totais
- **AND** sai com código 0

### Requirement: Saída de metadados em JSON
O sistema SHALL emitir os metadados como um único objeto JSON em stdout quando `--json` for informado, com um esquema estável e documentado, independente do formato bruto do ffprobe.

#### Scenario: Consumo por script
- **WHEN** o usuário executa `nago info video.mp4 --json`
- **THEN** stdout contém apenas JSON válido com os campos `container`, `duration_secs`, `size_bytes`, `bit_rate` e `streams`

### Requirement: Detecção de HDR
O sistema SHALL identificar vídeo HDR (função de transferência PQ/SMPTE 2084 ou HLG/ARIB STD-B67) e indicá-lo nos metadados.

#### Scenario: Vídeo HDR10
- **WHEN** o usuário inspeciona um vídeo HEVC 10-bit com transferência `smpte2084` e primárias `bt2020`
- **THEN** o stream de vídeo é marcado como `hdr: "HDR10"`

#### Scenario: Vídeo SDR
- **WHEN** o usuário inspeciona um vídeo com transferência `bt709`
- **THEN** o stream de vídeo é marcado como `hdr: null`

### Requirement: Erro para entrada inválida
O sistema MUST rejeitar com mensagem clara e código de saída diferente de zero um caminho inexistente ou um arquivo que não contém streams de mídia reconhecíveis.

#### Scenario: Arquivo inexistente
- **WHEN** o usuário executa `nago info nao-existe.mp4`
- **THEN** o sistema informa que o arquivo não foi encontrado e sai com código 1

#### Scenario: Arquivo que não é mídia
- **WHEN** o usuário executa `nago info notas.txt`
- **THEN** o sistema informa que o arquivo não é uma mídia suportada e sai com código 1
