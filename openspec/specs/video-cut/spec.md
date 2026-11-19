# video-cut Specification

## Purpose

Extrair um trecho de um vídeo por intervalo de tempo, oferecendo um modo instantâneo sem perda (stream copy) e um modo preciso ao frame quando a exatidão importa mais que a velocidade.

## Requirements

### Requirement: Especificação do intervalo de corte
O comando `nago cut <arquivo>` SHALL aceitar `--from` e uma entre `--to` ou `--duration`, nos formatos `SS[.ms]`, `MM:SS[.ms]` e `HH:MM:SS[.ms]`. A ausência de `--from` significa o início e a ausência de `--to`/`--duration` significa o fim do arquivo.

#### Scenario: Intervalo com --to
- **WHEN** o usuário executa `nago cut aula.mp4 --from 1:30 --to 2:45.5`
- **THEN** a saída contém o trecho de 00:01:30.000 a 00:02:45.500

#### Scenario: Intervalo com --duration
- **WHEN** o usuário executa `nago cut aula.mp4 --from 90 --duration 30`
- **THEN** a saída contém 30 segundos a partir de 00:01:30

### Requirement: Validação do intervalo
O sistema MUST rejeitar, antes de processar, intervalos com início maior ou igual ao fim, valores negativos, formatos inválidos, uso simultâneo de `--to` e `--duration`, ou início além da duração do arquivo.

#### Scenario: Início depois do fim
- **WHEN** o usuário executa `nago cut a.mp4 --from 2:00 --to 1:00`
- **THEN** o sistema informa que o início deve ser anterior ao fim e sai com código 2 sem criar arquivo

#### Scenario: Início além da duração
- **WHEN** o usuário pede `--from 10:00` em um vídeo de 5 minutos
- **THEN** o sistema informa a duração real do arquivo e sai com código 2

#### Scenario: Fim além da duração
- **WHEN** o usuário pede `--to 10:00` em um vídeo de 5 minutos
- **THEN** o corte vai até o fim do arquivo e o sistema emite um aviso

### Requirement: Corte rápido sem recodificação
Por padrão, o corte SHALL copiar os streams sem recodificar, iniciando no keyframe mais próximo anterior ou igual a `--from`, e SHALL informar o instante real de início quando ele diferir do pedido.

#### Scenario: Início fora de keyframe
- **WHEN** o usuário corta a partir de 00:01:30 e o keyframe anterior mais próximo está em 00:01:28.4
- **THEN** a saída começa em 00:01:28.4, nenhum stream é recodificado
- **AND** o sistema avisa que o início real é 00:01:28.4 e sugere `--exact`

### Requirement: Corte exato ao frame
Com `--exact`, o corte SHALL começar e terminar com precisão de um frame em relação aos tempos pedidos, recodificando o vídeo com o perfil de qualidade `visually-lossless` e mantendo áudio e vídeo sincronizados.

#### Scenario: Corte exato
- **WHEN** o usuário executa `nago cut a.mp4 --from 1:30 --to 1:40 --exact` em um vídeo de 30 fps
- **THEN** a duração da saída difere de 10 s em no máximo 1 frame (33 ms)
- **AND** o primeiro frame corresponde ao instante 00:01:30 do original

### Requirement: Preservação de streams no corte
O corte SHALL preservar, por padrão, todos os streams de vídeo, áudio e legenda e os metadados de idioma e disposição do arquivo original.

#### Scenario: Arquivo com várias faixas
- **WHEN** o usuário corta um MKV com 2 faixas de áudio e 2 de legenda
- **THEN** a saída contém as mesmas 2 faixas de áudio e 2 de legenda com os mesmos idiomas

### Requirement: Nome padrão da saída do corte
Sem `-o`, a saída SHALL ser gravada no mesmo diretório da entrada, com o nome `<nome>.cut.<ext>` e o mesmo container da entrada.

#### Scenario: Nome padrão
- **WHEN** o usuário executa `nago cut ~/v/aula.mp4 --from 10 --to 20`
- **THEN** a saída é `~/v/aula.cut.mp4`
