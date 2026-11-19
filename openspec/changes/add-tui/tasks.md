# Tasks

## 1. Preparação no core (D3, D4, D5)

- [x] 1.1 Mover a formatação de `nago-cli/src/render.rs` para `nago_core::display` (linhas estruturadas, sem ANSI) e fazer a CLI usá-la; verificar que os snapshots `insta` de `info` e dos relatórios continuam idênticos sem `INSTA_UPDATE`
- [x] 1.2 Adicionar `Reservations` a `output::resolve` (ocupado = existe no disco ou reservado; caminho explícito reservado → erro de uso); verificar com testes unitários dos cenários "Mesmo arquivo comprimido duas vezes" e "Caminho explícito repetido" e com os testes de saída existentes passando
- [x] 1.3 Separar `ops::prepare_{compress,convert,cut}` (aceitando `MediaInfo` já lido e `Reservations`) de `ops::execute(runner, Prepared)` e reescrever `compress`/`convert`/`cut` como `prepare` + `execute`; verificar que toda a suíte da CLI passa sem mudanças e que um teste unitário obtém de `prepare_convert` o mesmo erro de `nago convert --to m4a` num arquivo sem áudio, sem criar arquivos

## 2. Prévia no core (D6, D7)

- [x] 2.1 Implementar `preview::keyframe_index` (ffprobe de pacotes com flag `K`) com busca de keyframe anterior/seguinte; verificar na fixture com GOP de 2 s que o anterior a 5,3 s é 4,0 s e o seguinte é 6,0 s, e que concorda com `probe::keyframe_at_or_before` em 20 instantes
- [x] 2.2 Implementar `preview::frame_png` (seek, `scale` mantendo proporção, autorotate, tonemap HDR com `zscale` quando disponível, cancelável); verificar com testes que o PNG da fixture 1080p cabe em 320x180 com a proporção certa, que a fixture HDR10 gera PNG 8-bit sem erro e que cancelar a task mata o ffmpeg
- [x] 2.3 Detectar `zscale` nas capabilities (campo novo no cache, com invalidação pela versão do cache); verificar com o teste de parse de filtros sobre a saída gravada do FFmpeg 9

## 3. Esqueleto da TUI e entrada (D1, D2, D9)

- [x] 3.1 Criar o crate `nago-tui` com ratatui 0.30, crossterm 0.29 (`event-stream`) e `ratatui-image` (última 11.x estável), contendo `App`, `Msg`, `Effect`, `update`, `view` vazios e `run(TuiOptions) -> Result<TuiExit>`; verificar que `cargo build` e `cargo lint` passam
- [x] 3.2 Implementar o loop (`select!` de terminal, canal de `Msg` e tick), o guard de restauração do terminal, o panic hook e o log em arquivo (`$XDG_STATE_HOME/nago/tui.log`); verificar com um teste que força pânico dentro do loop num pseudo-terminal e confirma saída 1 com o terminal restaurado (sequência de saída da tela alternativa emitida)
- [x] 3.3 Tornar o subcomando opcional na CLI (`None` + TTY → TUI; sem TTY → ajuda e código 2), adicionar `nago tui [arquivos...]` (sem TTY → código 2) e recusar `--json` com a TUI; verificar com testes `assert_cmd` (sem TTY) e com um teste em pseudo-terminal que `nago` abre a tela inicial e sai com `q` e código 0

## 4. Navegador e metadados (spec interactive-mode)

- [x] 4.1 Implementar o navegador (ordenação, filtro de extensões, ocultar `.*` e `*.nago-*.part.*`, entrar/voltar, mostrar todos, marcação múltipla) com leitura de diretório como efeito; verificar com testes de `update` e um snapshot `TestBackend` do cenário "Filtro de extensões"
- [x] 4.2 Implementar o painel de metadados com probe em debounce de 150 ms e cache por (caminho, mtime, tamanho), mostrando erro de leitura no painel; verificar com testes de `update` (debounce emite um só probe ao rolar) e snapshots do painel para a fixture H.264 e para um `.mp4` corrompido
- [x] 4.3 Implementar a tela de ajuda `?` com os atalhos da tela atual (tabela D10); verificar com snapshots da ajuda no navegador, no corte e na fila

## 5. Formulários das operações (spec interactive-mode)

- [x] 5.1 Implementar o componente de formulário (campos de texto, seleção, alternância, erro por campo) e o envio que gera um job por arquivo marcado; verificar com testes de `update` cobrindo navegação entre campos, erro bloqueando o envio e o cenário "Seleção múltipla"
- [x] 5.2 Implementar os formulários de compressão e conversão chamando `prepare` em debounce de 300 ms e mostrando saída, perfil, codec e encoder; verificar com testes os cenários "Padrões da compressão", "GPU indisponível" (capacidades simuladas) e "Conversão para formato sem áudio", e com snapshots dos dois formulários

## 6. Corte com prévia (spec frame-preview)

- [x] 6.1 Implementar o formulário de corte: campos de início/fim, passos de 1 frame/1 s/10 s, alternância início/fim, validação via `prepare_cut` e modo rápido/exato; verificar com testes de `update` os cenários "Passo de um frame", "Intervalo inválido" e "Corte de áudio"
- [x] 6.2 Integrar o índice de keyframes (efeito ao abrir, estado "indexando", saltos `[`/`]`, indicação do keyframe e sugestão do modo exato acima de 1 s); verificar com testes de `update` os cenários "Ajuste por keyframe" e "Keyframe distante" usando um índice simulado
- [x] 6.3 Integrar a miniatura: detecção de protocolo (halfblocks → desativa; `NAGO_TUI_IMAGES` força), geração por pedido, debounce de 120 ms, cancelamento do pedido obsoleto e cache LRU de 32 frames; verificar com testes de `update` que 30 ajustes seguidos produzem um único frame exibido (o último) e que, sem protocolo, nenhum efeito de extração é emitido, e conferir manualmente no kitty com a gravação de celular e com a fixture HDR10

## 7. Fila de jobs (spec job-queue)

- [x] 7.1 Implementar `Queue`/`Job` com os estados de D8 e a política "próximo pendente", incluindo reservas de saída e o `prepare` repetido antes de executar; verificar com testes de `update` os cenários "Enfileirar durante a execução", "Remover pendente" e "Mesmo arquivo comprimido duas vezes"
- [x] 7.2 Executar jobs pelo `EffectRunner` (Runner com canal e token por job, `JobEvent` → `Msg`, `NotSmaller` → descartado, `Cancelled` → cancelado); verificar com um teste de integração que roda três jobs reais sobre as fixtures (corte, conversão e compressão `small`) e confere os estados finais e os arquivos gerados
- [x] 7.3 Implementar a tela da fila (estado, fase, porcentagem, velocidade, ETA e relatório do job selecionado via `nago_core::display`, com erro e fim do log nos jobs que falharam); verificar com snapshots de um job em execução, um concluído com aviso de VMAF e um que falhou (ffmpeg falso como no teste da CLI)
- [x] 7.4 Implementar o cancelamento do job atual (`x`) e o seguimento da fila; verificar com teste de integração que cancelar uma compressão real com um job pendente não deixa arquivo parcial e inicia o pendente

## 8. Saída e sinais (spec job-queue)

- [x] 8.1 Implementar a confirmação de saída com contagem de jobs, Ctrl+C duplo em até 2 s, SIGTERM e os códigos 0/130 de D9; verificar com testes de `update` os códigos de cada caminho e com um teste em pseudo-terminal que SIGTERM durante uma compressão sai com 130 sem arquivo parcial

## 9. Documentação e verificação final

- [x] 9.1 Documentar a TUI no README (como abrir, telas, tabela de atalhos, miniatura e terminais suportados, `NAGO_TUI_IMAGES`); verificar que todos os atalhos da tabela D10 aparecem na ajuda `?` comparando com os snapshots da tarefa 4.3
- [x] 9.2 Rodar o fluxo completo na TUI nesta máquina (kitty): abrir `~/Videos`, cortar com miniatura, enfileirar compressão com verificação e conversão, cancelar e sair; verificar que `cargo test --workspace -- --include-ignored`, `cargo lint` e `openspec validate add-tui --strict` passam

## Workflow follow-up

- Arquivar com `/openspec-archive-change add-tui` após a implementação.
