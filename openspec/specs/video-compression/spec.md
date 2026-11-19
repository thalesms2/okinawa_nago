# video-compression Specification

## Purpose

Reduzir o tamanho de vídeos recodificando-os com codecs modernos e perfis de qualidade pré-calibrados, mantendo a qualidade perceptualmente igual à original e permitindo medir objetivamente o resultado.

## Requirements

### Requirement: Perfis de qualidade
O comando `nago compress <arquivo>` SHALL aceitar `--quality` com os perfis `visually-lossless`, `high` (padrão), `balanced` e `small`, cada um com uma meta de VMAF médio documentada (respectivamente >= 95, >= 93, >= 91 e >= 88) e parâmetros de encoder calibrados para atingi-la em conteúdo típico.

#### Scenario: Perfil padrão
- **WHEN** o usuário executa `nago compress video.mp4` sem `--quality`
- **THEN** o perfil `high` é usado e o perfil aplicado aparece no relatório final

### Requirement: Escolha de codec
O sistema SHALL aceitar `--codec av1|hevc|h264`, com `av1` como padrão. O vídeo de saída MUST manter a resolução, a taxa de quadros e a orientação da entrada.

#### Scenario: Codec HEVC
- **WHEN** o usuário executa `nago compress video.mp4 --codec hevc`
- **THEN** o vídeo de saída é HEVC com a mesma resolução e fps da entrada

### Requirement: Preservação de HDR e profundidade de cor
Vídeos HDR SHALL manter função de transferência, primárias, matriz e metadados de luminância (mastering display e MaxCLL quando presentes). Se o codec ou encoder escolhido não suportar HDR, o sistema MUST recusar a operação com explicação, em vez de gerar SDR silenciosamente.

#### Scenario: HDR10 para AV1
- **WHEN** o usuário comprime um vídeo HDR10 com `--codec av1`
- **THEN** a saída é AV1 10-bit com transferência `smpte2084` e primárias `bt2020`

#### Scenario: HDR com H.264
- **WHEN** o usuário comprime um vídeo HDR10 com `--codec h264`
- **THEN** o sistema explica que H.264 não preservará o HDR, sugere `av1` ou `hevc` e sai com código 2

### Requirement: Tratamento de áudio e legendas na compressão
Por padrão, a compressão SHALL copiar áudio e legendas sem recodificar. Com `--audio-bitrate <kbps>`, o áudio SHALL ser recodificado para Opus (ou AAC quando o container exigir) nesse bitrate.

#### Scenario: Áudio padrão
- **WHEN** o usuário comprime um vídeo com áudio AAC sem opções de áudio
- **THEN** o áudio da saída é idêntico ao da entrada

### Requirement: Container de saída da compressão
Sem `-o`, a saída SHALL usar o mesmo container da entrada quando ele suportar o codec escolhido, ou `mkv` caso contrário, com o nome `<nome>.compressed.<ext>` no diretório da entrada.

#### Scenario: AVI de entrada
- **WHEN** o usuário comprime `antigo.avi` com o codec padrão
- **THEN** a saída é `antigo.compressed.mkv`

### Requirement: Proteção contra saída maior que a entrada
Se o arquivo comprimido ficar maior ou igual à entrada, o sistema MUST descartar a saída, manter o original intacto, avisar o usuário e sair com código 3, a menos que `--force` seja informado.

#### Scenario: Vídeo já muito comprimido
- **WHEN** o usuário comprime um vídeo AV1 de baixo bitrate e o resultado fica 10% maior
- **THEN** nenhum arquivo de saída permanece, o sistema avisa que a compressão não reduziria o tamanho e sai com código 3

### Requirement: Verificação objetiva de qualidade
Com `--verify`, o sistema SHALL calcular o VMAF médio e o percentil 5 da saída em relação à entrada e incluí-los no relatório, avisando quando a média ficar abaixo da meta do perfil. Sem libvmaf, `--verify` MUST falhar antes de codificar.

#### Scenario: Verificação bem-sucedida
- **WHEN** o usuário executa `nago compress video.mp4 --verify`
- **THEN** o relatório final inclui `vmaf_mean` e `vmaf_p5`

#### Scenario: Qualidade abaixo da meta
- **WHEN** a verificação resulta em VMAF médio 90 com o perfil `high`
- **THEN** a saída é mantida e o sistema avisa que a meta 93 não foi atingida, sugerindo o perfil `visually-lossless`

#### Scenario: FFmpeg sem libvmaf
- **WHEN** o usuário usa `--verify` com um ffmpeg compilado sem libvmaf
- **THEN** o sistema informa que a libvmaf é necessária e sai com código 1 antes de iniciar a codificação
