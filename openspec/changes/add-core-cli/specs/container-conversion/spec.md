# Spec Delta

## Purpose

Trocar o container (formato) de um arquivo de mídia preservando os streams originais sempre que possível, recodificando apenas o estritamente necessário para que o resultado seja válido no formato de destino.

## ADDED Requirements

### Requirement: Formatos de destino suportados
O comando `nago convert <arquivo> --to <formato>` SHALL suportar os containers de vídeo `mp4`, `mkv`, `mov` e `webm` e os formatos somente de áudio `m4a`, `mp3`, `opus` e `flac`. O formato também SHALL ser inferido da extensão de `-o` quando `--to` for omitido.

#### Scenario: Formato inferido da saída
- **WHEN** o usuário executa `nago convert video.mkv -o video.mp4`
- **THEN** o destino é tratado como `mp4`

#### Scenario: Formato não suportado
- **WHEN** o usuário executa `nago convert video.mkv --to avi`
- **THEN** o sistema lista os formatos suportados e sai com código 2

### Requirement: Remux sem recodificação
Quando todos os streams forem compatíveis com o container de destino, a conversão SHALL copiá-los sem recodificar.

#### Scenario: MKV com H.264 e AAC para MP4
- **WHEN** o usuário converte um MKV com vídeo H.264 e áudio AAC para `mp4`
- **THEN** nenhum stream é recodificado e o vídeo e o áudio da saída são idênticos bit a bit aos da entrada

### Requirement: Recodificação seletiva de streams incompatíveis
Quando um stream for incompatível com o destino, o sistema SHALL recodificar apenas esse stream para um codec compatível, informando quais streams foram recodificados e por quê.

#### Scenario: Áudio PCM para MP4
- **WHEN** o usuário converte um MOV com vídeo H.264 e áudio PCM para `mp4`
- **THEN** o vídeo é copiado, o áudio é recodificado para AAC e o sistema informa a recodificação do áudio

#### Scenario: H.264 para WebM
- **WHEN** o usuário converte um MP4 com vídeo H.264 e áudio AAC para `webm`
- **THEN** o vídeo é recodificado para AV1 com o perfil de qualidade padrão e o áudio para Opus

### Requirement: Tratamento de legendas na conversão
Legendas de texto SHALL ser convertidas para o formato de legenda aceito pelo destino. Legendas que o destino não suporta MUST ser descartadas com aviso explícito por faixa.

#### Scenario: Legenda de imagem para MP4
- **WHEN** o usuário converte um MKV com legenda PGS para `mp4`
- **THEN** a faixa PGS é descartada e o sistema avisa qual faixa foi descartada e por quê

### Requirement: Extração de áudio
Quando o destino for um formato somente de áudio, o sistema SHALL extrair a primeira faixa de áudio (ou a indicada por `--audio-track <n>`), copiando-a quando compatível e recodificando-a caso contrário.

#### Scenario: Extrair AAC para m4a
- **WHEN** o usuário executa `nago convert video.mp4 --to m4a` em um arquivo com áudio AAC
- **THEN** a saída contém apenas o áudio AAC copiado, sem recodificação

### Requirement: MP4 otimizado para streaming
Saídas `mp4`, `m4a` e `mov` SHALL ter o índice (moov atom) no início do arquivo, permitindo reprodução progressiva.

#### Scenario: Reprodução na web
- **WHEN** o usuário converte para `mp4`
- **THEN** o átomo `moov` aparece antes do átomo `mdat` no arquivo de saída
