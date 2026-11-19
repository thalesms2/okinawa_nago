# Spec Delta

## Purpose

Oferecer um modo interativo no terminal para escolher arquivos, entender o que eles contêm e configurar cortes, conversões e compressões sem decorar flags, com os mesmos padrões e garantias da CLI.

## ADDED Requirements

### Requirement: Entrada no modo interativo
Executar `nago` sem subcomando num terminal interativo (stdin e stdout ligados a um TTY) SHALL abrir a TUI no diretório atual. `nago tui [arquivos...]` SHALL abrir a TUI com os arquivos informados já selecionados. Sem TTY, `nago` sem subcomando MUST manter o comportamento da CLI (mostrar a ajuda e sair com código 2), e `nago tui` MUST recusar com explicação e código 2.

#### Scenario: nago sem argumentos no terminal
- **WHEN** o usuário executa `nago` num terminal interativo
- **THEN** a TUI abre mostrando o navegador de arquivos no diretório atual

#### Scenario: nago sem argumentos em pipe
- **WHEN** um script executa `nago | cat`
- **THEN** a ajuda é exibida e o código de saída é 2, sem abrir a TUI

#### Scenario: Arquivos na linha de comando
- **WHEN** o usuário executa `nago tui a.mp4 b.mkv`
- **THEN** a TUI abre com os dois arquivos selecionados e o painel de metadados do primeiro

### Requirement: Navegador de arquivos
A TUI SHALL listar diretórios e arquivos de mídia do diretório atual, permitindo entrar em pastas, voltar ao diretório pai e marcar vários arquivos. Por padrão, apenas extensões de vídeo e áudio conhecidas SHALL ser exibidas, com uma tecla para mostrar todos os arquivos. Arquivos temporários do próprio `nago` (`.*.nago-*.part.*`) MUST ficar ocultos.

#### Scenario: Filtro de extensões
- **WHEN** a pasta contém `video.mp4`, `notas.txt` e `musica.flac`
- **THEN** a lista mostra `video.mp4` e `musica.flac`, e `notas.txt` aparece só depois de ativar "mostrar todos"

#### Scenario: Seleção múltipla
- **WHEN** o usuário marca três arquivos e escolhe "comprimir"
- **THEN** o formulário de compressão se aplica aos três, gerando um job por arquivo

### Requirement: Painel de metadados
Ao destacar um arquivo, a TUI SHALL mostrar as mesmas informações do `nago info` (container, duração, tamanho, bitrate e streams com codec, resolução, fps, HDR e idioma). A leitura MUST acontecer sem travar a navegação. Um arquivo que o ffprobe não consegue ler SHALL exibir o motivo no painel em vez de encerrar a TUI.

#### Scenario: Arquivo inválido
- **WHEN** o usuário destaca um arquivo `.mp4` corrompido
- **THEN** o painel mostra que o arquivo não pôde ser lido e por quê, e a navegação continua funcionando

### Requirement: Formulários das operações
A TUI SHALL oferecer formulários para cortar, converter e comprimir com as mesmas opções, padrões e validações dos comandos equivalentes da CLI. Cada formulário MUST mostrar, antes de enfileirar, o caminho de saída resultante e, na compressão, o perfil, o codec e o encoder que será usado (software ou GPU, conforme a detecção de capacidades). Opções inválidas SHALL ser apontadas no próprio campo e impedir o envio, com a mesma mensagem que a CLI daria.

#### Scenario: Padrões da compressão
- **WHEN** o usuário abre o formulário de compressão sem mudar nada
- **THEN** o perfil é `high`, o codec é `av1`, o encoder mostrado é o de software e a saída é `<nome>.compressed.<ext>`

#### Scenario: GPU indisponível
- **WHEN** o usuário ativa "usar GPU" numa máquina sem encoder de hardware funcional
- **THEN** o formulário avisa que a GPU não está disponível e mostra o encoder de software que será usado

#### Scenario: Conversão para formato sem áudio
- **WHEN** o usuário escolhe converter para `m4a` um vídeo sem faixa de áudio
- **THEN** o formulário mostra o mesmo erro que `nago convert --to m4a` mostraria e não permite enfileirar

### Requirement: Teclado e ajuda
Toda ação da TUI SHALL ser acessível pelo teclado, e a tecla `?` SHALL mostrar os atalhos da tela atual. A TUI MUST restaurar o estado do terminal (modo normal, cursor visível, tela principal) ao sair, inclusive após erro interno ou pânico.

#### Scenario: Saída após pânico
- **WHEN** ocorre um erro interno inesperado durante a TUI
- **THEN** o terminal volta ao modo normal, a mensagem de erro é impressa e o código de saída é 1
