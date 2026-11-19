# frame-preview Specification

## Purpose

Permitir escolher o início e o fim de um corte vendo onde se está no vídeo: miniatura do frame, keyframes vizinhos e ajuste fino por teclado, deixando claro o que o corte rápido e o exato vão produzir.

## Requirements

### Requirement: Seleção de início e fim
O formulário de corte SHALL aceitar início e fim digitados nos mesmos formatos da CLI (`SS[.ms]`, `MM:SS[.ms]`, `HH:MM:SS[.ms]`) e SHALL permitir ajustá-los pelo teclado em passo fino (1 frame), médio (1 s) e grosso (10 s), além de saltar para o keyframe anterior ou seguinte. As validações de intervalo MUST ser as mesmas do `nago cut`.

#### Scenario: Ajuste por keyframe
- **WHEN** o início está em 00:01:07.000 e o usuário pede "próximo keyframe"
- **THEN** o início passa para o primeiro keyframe depois de 00:01:07.000

#### Scenario: Passo de um frame
- **WHEN** o vídeo tem 30 fps, o fim está em 00:00:10.000 e o usuário avança um frame
- **THEN** o fim passa para 00:00:10.033

#### Scenario: Intervalo inválido
- **WHEN** o usuário ajusta o início para depois do fim
- **THEN** o formulário mostra que o início deve ser anterior ao fim e não permite enfileirar

### Requirement: Indicação do keyframe no corte rápido
No modo rápido, o formulário SHALL mostrar o instante real em que o trecho vai começar (o keyframe em ou antes do início pedido) e a diferença em relação ao início pedido. Quando a diferença passar de 1 segundo, SHALL sugerir o modo exato.

#### Scenario: Keyframe distante
- **WHEN** o usuário pede início em 00:00:42.000, o keyframe anterior está em 00:00:38.500 e o modo é rápido
- **THEN** o formulário informa que o trecho começará em 00:00:38.500 (3,5 s antes) e sugere o modo exato

### Requirement: Miniatura do frame
Em terminais com protocolo de imagem (kitty, sixel ou iTerm2), o formulário de corte SHALL mostrar uma miniatura do frame no instante selecionado (início ou fim, conforme o campo em foco), mantendo a proporção e respeitando rotação e HDR com mapeamento para SDR na exibição. A miniatura MUST ser atualizada sem bloquear a digitação e o ajuste.

#### Scenario: Terminal com suporte a imagem
- **WHEN** o usuário ajusta o início para 00:02:00.000 num terminal kitty
- **THEN** a miniatura passa a mostrar o frame de 00:02:00.000

### Requirement: Terminal sem protocolo de imagem
Em terminais sem protocolo de imagem, a TUI SHALL omitir a miniatura e mostrar apenas o tempo e o keyframe, sem erro e sem substituir a imagem por arte em caracteres.

#### Scenario: Terminal sem suporte a imagem
- **WHEN** a TUI roda num terminal sem protocolo de imagem
- **THEN** o formulário de corte funciona normalmente, exibindo os tempos e o keyframe sem miniatura

### Requirement: Atualização da miniatura sob ajustes rápidos
Quando o instante muda várias vezes em sequência, a TUI SHALL exibir apenas o frame do último instante pedido, descartando extrações já obsoletas, e MUST continuar respondendo ao teclado durante as extrações.

#### Scenario: Ajustes em sequência
- **WHEN** o usuário segura a tecla de avançar 1 s por vários segundos
- **THEN** a interface continua respondendo e, ao soltar, a miniatura mostra o frame do instante final

### Requirement: Arquivos sem vídeo
Para arquivos só de áudio, o formulário de corte SHALL funcionar sem miniatura e sem salto de keyframe, com o ajuste em passos de tempo.

#### Scenario: Corte de áudio
- **WHEN** o usuário abre o corte de um arquivo `.flac`
- **THEN** o formulário mostra início e fim ajustáveis por tempo, sem miniatura e sem as ações de keyframe
