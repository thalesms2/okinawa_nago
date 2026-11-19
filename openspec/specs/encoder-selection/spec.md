# encoder-selection Specification

## Purpose

Detectar o que o FFmpeg instalado e o hardware da máquina conseguem fazer e escolher o encoder adequado, privilegiando eficiência de compressão por padrão e velocidade por GPU quando pedido.

## Requirements

### Requirement: Verificação do FFmpeg
Antes de qualquer operação de mídia, o sistema MUST verificar se `ffmpeg` e `ffprobe` estão disponíveis (no PATH ou em `NAGO_FFMPEG`/`NAGO_FFPROBE`) e na versão 6.0 ou superior.

#### Scenario: FFmpeg ausente
- **WHEN** o usuário executa qualquer comando de mídia sem ffmpeg instalado
- **THEN** o sistema informa que o ffmpeg é necessário, mostra como instalá-lo e sai com código 1

#### Scenario: FFmpeg antigo
- **WHEN** o ffmpeg instalado é da versão 5.1
- **THEN** o sistema informa a versão encontrada e a mínima exigida e sai com código 1

### Requirement: Diagnóstico de capacidades
O sistema SHALL fornecer `nago doctor`, que mostra a versão do ffmpeg, os encoders AV1/HEVC/H.264 de software e hardware disponíveis, se cada encoder de hardware de fato funciona nesta máquina, e se a libvmaf está presente.

#### Scenario: Máquina com NVIDIA
- **WHEN** o usuário executa `nago doctor` em uma máquina com GPU NVIDIA e driver funcional
- **THEN** `av1_nvenc`, `hevc_nvenc` e `h264_nvenc` aparecem como funcionais

#### Scenario: Encoder listado mas inutilizável
- **WHEN** o ffmpeg lista `hevc_vaapi`, mas não há dispositivo VAAPI utilizável
- **THEN** `hevc_vaapi` aparece como indisponível, com o motivo

### Requirement: Encoder de software por padrão
Por padrão, o sistema SHALL usar encoders de software (SVT-AV1 para AV1, x265 para HEVC, x264 para H.264), que oferecem o menor tamanho para a mesma qualidade.

#### Scenario: Compressão padrão
- **WHEN** o usuário executa `nago compress video.mp4`
- **THEN** o encoder usado é o de software do codec escolhido e ele aparece no relatório

### Requirement: Aceleração por GPU opcional
Com `--gpu`, o sistema SHALL usar um encoder de hardware funcional para o codec escolhido, na ordem de preferência NVENC, QSV, VAAPI. Se nenhum funcionar, o sistema SHALL avisar e usar o encoder de software.

#### Scenario: GPU disponível
- **WHEN** o usuário executa `nago compress video.mp4 --gpu` em máquina com NVENC funcional
- **THEN** o encoder usado é `av1_nvenc`

#### Scenario: GPU indisponível
- **WHEN** o usuário usa `--gpu` em máquina sem encoder de hardware funcional
- **THEN** o sistema avisa que vai usar o encoder de software e prossegue com ele

### Requirement: Encoder ausente
Se nenhum encoder (software ou hardware) estiver disponível para o codec pedido, o sistema MUST falhar antes de iniciar, indicando codecs alternativos disponíveis.

#### Scenario: FFmpeg sem SVT-AV1
- **WHEN** o usuário pede `--codec av1` e o ffmpeg não tem nenhum encoder AV1 funcional
- **THEN** o sistema informa que não há encoder AV1, sugere `--codec hevc` e sai com código 1
