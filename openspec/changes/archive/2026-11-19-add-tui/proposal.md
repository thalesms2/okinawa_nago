# Proposal

## Why

A CLI do `nago` já cobre corte, conversão e compressão, mas o usuário precisa saber os caminhos, os tempos exatos do corte e as opções de cada comando. Para o corte isso pesa mais, porque escolher início e fim sem ver o vídeo é tentativa e erro. Um modo interativo no terminal deixa o fluxo "escolher arquivo → ver o que ele tem → configurar → acompanhar" acessível sem decorar flags. Ele reaproveita o `nago-core` e os perfis já calibrados.

## What Changes

- Novo crate `nago-tui` (biblioteca, ratatui + crossterm) usado pelo binário `nago`, que continua sendo um só.
- `nago` sem subcomando, num terminal interativo, abre a TUI. Fora de um TTY, mantém o comportamento atual (ajuda e código 2). `nago tui [arquivos...]` abre a TUI já com os arquivos selecionados.
- Navegador de arquivos com filtro de extensões de vídeo e painel de metadados do arquivo selecionado (o mesmo conteúdo do `nago info`).
- Formulários para as três operações (cortar, converter, comprimir) com os mesmos padrões e validações da CLI. O caminho de saída e o encoder escolhido ficam visíveis antes de enfileirar.
- Corte com seleção de início e fim por campos de tempo e ajuste por teclado, com passo fino, passo grosso e salto de keyframe em keyframe. Mostra o keyframe em que o corte rápido vai começar e uma miniatura do frame em terminais com protocolo de imagem (kitty, sixel, iTerm2).
- Fila sequencial de jobs: o usuário enfileira várias operações (em um ou mais arquivos), e elas rodam uma após a outra. Cada job mostra progresso, velocidade, ETA e o relatório final (tamanho, redução, VMAF). Dá para cancelar o job atual e remover pendentes.
- No `nago-core`: extração de um frame como imagem para a miniatura, e listagem de keyframes vizinhos de um instante.

## Capabilities

### New Capabilities
- `interactive-mode`: entrada da TUI, navegação de arquivos, painel de metadados e formulários das operações.
- `frame-preview`: escolha visual do trecho do corte (miniatura do frame, keyframes vizinhos e ajuste fino por teclado).
- `job-queue`: fila sequencial de jobs na TUI, com estados, progresso, cancelamento e relatório por job.

### Modified Capabilities
- Nenhuma. Os comandos da CLI e seus códigos de saída não mudam. `nago` sem subcomando fora de um TTY continua saindo com código 2.

## Fora do escopo

- Execução paralela, processamento de pastas inteiras e retomada da fila após fechar o programa (change futura `add-batch-queue`).
- Reprodução de vídeo ou áudio dentro do terminal. A TUI mostra apenas frames estáticos.
- Miniatura em terminais sem protocolo de imagem: neles a TUI mostra só os tempos e o keyframe, sem arte em blocos de texto.
- Busca automática de CRF por meta de VMAF (change futura `add-quality-targeting`).
- Perfis personalizados e arquivo de configuração (change futura `add-user-config`).
- Suporte a mouse além do básico do terminal; a navegação é por teclado.
- Interface gráfica (janela nativa) ou web.

## Impact

- **Código:** novo crate `crates/nago-tui`. Em `nago-cli`, o subcomando passa a ser opcional e surge o subcomando `tui`. Em `nago-core`, novo módulo `preview` (extração de frame e keyframes vizinhos). Nenhuma mudança nas APIs existentes de `ops`/`runner`.
- **Dependências novas:** `ratatui`, `crossterm` (com `event-stream`), `ratatui-image` (protocolos kitty/sixel/iTerm2) e `image` (decodificar o PNG do frame).
- **Testes:** snapshots das telas com o `TestBackend` do ratatui e testes da máquina de estados sem terminal. A fila é testada executando jobs reais sobre as fixtures sintéticas.
- **Documentação:** seção da TUI no README, com atalhos de teclado.
