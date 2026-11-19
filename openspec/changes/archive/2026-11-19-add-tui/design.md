# Design

## Context

A motivação está em proposal.md. Estado atual relevante:

- **`nago-core`** expõe `ops::{compress, convert, cut}`. Cada uma recebe um `*Request` e faz tudo numa chamada: probe, plano, resolução da saída, execução e relatório. Erros de validação (`Error::Usage`) só aparecem dentro dessas funções, depois do probe.
- **`Runner`** emite `JobEvent` (`Phase`, `Progress`, `Warning`) por um `UnboundedSender` e aceita um `CancellationToken`. O cancelamento já encerra o ffmpeg (`q` → SIGTERM → SIGKILL), e o `TempOutput` garante que nenhum arquivo parcial fique.
- **`nago-cli/src/render.rs`** formata `MediaInfo` e `Report` como texto. A TUI precisa do mesmo conteúdo.
- **`output::resolve`** escolhe o nome padrão com sufixo `-N` olhando apenas o disco.
- **`probe::keyframe_at_or_before`** consulta um keyframe por vez. O corte na TUI precisa de keyframes anteriores e seguintes repetidamente.
- **`clap`** exige subcomando hoje: `nago` sem argumentos sai com código 2 e mostra a ajuda.
- **FFmpeg 9** nesta máquina tem `zscale` e `tonemap`. O CI (Ubuntu 24.04, FFmpeg 6.1) também tem `zscale`.

## Goals / Non-Goals

**Goals:**
- Lógica da TUI testável sem terminal: estado e transições puras, efeitos isolados.
- Paridade com a CLI por construção: mesmas validações, nomes de saída e relatórios, vindos do mesmo código do core.
- Interface sempre responsiva: nada que chame ffmpeg/ffprobe roda na thread de desenho.

**Non-Goals:**
- Abstração de "backend de UI" para outras interfaces: a TUI fala direto com ratatui.
- Persistir estado entre sessões (diretório, fila, preferências).
- Temas de cores configuráveis: usa as cores do terminal (16 cores), com ênfase por negrito e inversão.

## Decisions

### D1. Crate `nago-tui` como biblioteca, binário único
`nago-tui` exporta `run(TuiOptions) -> Result<TuiExit>`, e `nago-cli` chama essa função no subcomando `tui` ou quando `nago` roda sem subcomando num TTY. `TuiExit` carrega o código de saída: 0, 130 ou 1.
- Na CLI, `Cli.command` passa a ser `Option<Command>`. Com `None` e `stdin`/`stdout` em TTY (`std::io::IsTerminal`), abre a TUI. Caso contrário, imprime a ajuda em stderr e sai com 2, preservando o comportamento atual e os testes de `cli_basics`.
- `--json` com a TUI é erro de uso (código 2).
- **Alternativa:** binário separado `nago-tui`. Rejeitada: o usuário teria dois comandos para lembrar, e o roadmap já previa `nago` sem argumentos.

### D2. Arquitetura Elm: `App` + `update` + `view`, com efeitos explícitos
- `App` guarda todo o estado (tela atual, navegador, formulário, fila, mensagens).
- `update(&mut App, Msg) -> Vec<Effect>` é pura: não faz I/O nem usa relógio. Recebe teclas, resultados de efeitos e eventos de job.
- `view(&App, &mut Frame)` desenha sem efeitos colaterais.
- Um `EffectRunner` assíncrono executa cada `Effect` numa task tokio e devolve o resultado como `Msg` por um canal. Efeitos: listar diretório, probe, indexar keyframes, extrair frame, iniciar job, cancelar job, sair.
- O loop principal faz `tokio::select!` sobre três fontes: eventos do terminal (`crossterm::event::EventStream`), o canal de `Msg` e um tick de 250 ms. O tick só redesenha quando há job em execução ou mensagem temporária.
- **Testes:** `update` é testada com sequências de `Msg`, verificando estado e efeitos. As telas são testadas com `ratatui::backend::TestBackend` e snapshots `insta`.
- **Alternativa:** componentes com estado próprio, cada um chamando o core direto. Rejeitada: espalha I/O pela UI e torna as telas difíceis de testar sem terminal.

### D3. `prepare` separado de `execute` no core
Para validar formulários e mostrar saída e encoder antes de enfileirar, cada operação ganha uma etapa de preparação:
- `ops::prepare_{compress,convert,cut}(tools, caps, &Request, &Reservations) -> Result<Prepared>` faz probe, plano e resolução da saída, sem executar. `Prepared` contém o plano, o caminho de saída, o encoder e os avisos.
- `ops::execute(runner, Prepared) -> Result<Report>` roda o plano.
- `compress`, `convert` e `cut` continuam existindo como `prepare` seguido de `execute`, então a CLI não muda e seus testes continuam valendo.
- O formulário chama `prepare` com debounce de 300 ms a cada mudança válida de campo. Um `Error::Usage` vira a mensagem de erro do campo, com o mesmo texto da CLI.
- O probe de cada arquivo fica em cache durante a sessão, por caminho, mtime e tamanho, e `prepare` o reaproveita. Para isso, `prepare` aceita um `MediaInfo` já lido.
- **Alternativa:** reimplementar as validações na TUI. Rejeitada: as mensagens divergiriam da CLI com o tempo, e a spec exige a mesma mensagem.

### D4. Reserva de nomes de saída na fila
`output::resolve` passa a receber um conjunto de caminhos reservados (`&Reservations`, por padrão vazio na CLI). Um caminho conta como ocupado se existe no disco ou está reservado.
- Ao enfileirar, o caminho resolvido entra nas reservas da fila. Um caminho explícito que já está reservado é recusado com erro de uso.
- Imediatamente antes de executar, o job chama `prepare` de novo com as reservas dos *outros* jobs. Assim um nome liberado por um job descartado ou cancelado pode ser reaproveitado, e um arquivo criado por fora nesse meio-tempo não é sobrescrito.
- O caminho real aparece no relatório do job. Pode diferir do mostrado no envio só nesses casos, e o relatório mostra os dois.

### D5. Formatação compartilhada no core
A formatação de `render.rs` (tamanho, bitrate, duração, linha de stream, relatório) vai para `nago_core::display`, com funções que devolvem linhas de texto estruturadas (rótulo e valor). Não há I/O nem códigos ANSI, o que respeita a regra de o core não ter I/O de terminal.
- A CLI concatena as linhas como hoje. Os snapshots de `info` e do relatório garantem que a saída não muda.
- A TUI converte as linhas em `ratatui::text::Line` com estilo.

### D6. Índice de keyframes por arquivo
Ao abrir o formulário de corte, um efeito roda `ffprobe -select_streams v:0 -show_entries packet=pts_time,flags -of csv=p=0` e guarda os `pts` dos pacotes com flag `K`, ordenados.
- Esse comando só faz demux, sem decodificar: leva segundos mesmo para filmes longos.
- Enquanto o índice não fica pronto, a TUI mostra "indexando keyframes…" e o salto de keyframe fica desabilitado. Passos de tempo e digitação funcionam desde o início.
- Com o índice, keyframe anterior e seguinte saem por busca binária, e a indicação do corte rápido é calculada localmente.
- A função fica em `nago_core::preview::keyframe_index(tools, info) -> Result<KeyframeIndex>`.
- `probe::keyframe_at_or_before` continua sendo usada pela CLI. Um teste garante que as duas concordam nas fixtures.
- **Alternativa:** consultar o ffprobe por janela a cada ajuste. Rejeitada: a latência por tecla seria perceptível, e o salto repetido faria uma consulta por tecla.

### D7. Miniatura: extração com ffmpeg e exibição com `ratatui-image`
- **Extração:** `nago_core::preview::frame_png(tools, info, t, max_w, max_h)` roda `ffmpeg -ss t -i arquivo -frames:v 1 -vf <filtros> -f image2pipe -c:v png -` e devolve os bytes do PNG.
  - Filtros: `scale` para caber em `max_w×max_h` em pixels, calculados pelo tamanho da célula informado pelo `ratatui-image`, mantendo a proporção.
  - Em HDR, aplica antes `zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=rgb24`.
  - Rotação: o autorotate padrão do ffmpeg aplica a matriz de rotação.
  - Sem `zscale` (detectado pelas capabilities), o HDR é exibido sem tonemap, com as cores lavadas, e a TUI avisa uma vez.
- **Concorrência:** cada pedido de miniatura leva um número de geração. Ao chegar um pedido novo, a task anterior é cancelada (o ffmpeg é morto) e o pedido espera 120 ms de debounce. Resultados de gerações antigas são descartados em `update`.
- **Cache:** um cache LRU de 32 frames por arquivo, com chave em milissegundos, evita reextrair ao ir e voltar.
- **Exibição:** `ratatui-image` detecta o protocolo do terminal (`Picker::from_query_stdio`) uma vez, após entrar no modo raw.
  - Se o protocolo detectado for `Halfblocks`, a miniatura é desativada, conforme a decisão de não usar arte em caracteres.
  - Dentro do tmux, a detecção pode falhar e cair em halfblocks, o que também desativa a miniatura.
  - A versão é a última estável compatível com ratatui 0.30 (11.x na data desta change; não usar a 12.0 rc).
- **Alternativa:** abrir o mpv externo. Rejeitada pelo usuário em favor da miniatura no terminal.

### D8. Fila sequencial no `nago-tui`
- `Queue` guarda `Vec<Job>` e o id do job ativo. Cada `Job` tem id, rótulo, `Request`, estado e relatório ou erro.
- **Estados:** `Pending`, `Running { phase, progress }`, `Done(Report)`, `Discarded(Report)`, `Failed(String, Option<log_tail>)` e `Cancelled`. O estado `Discarded` vem de `Error::NotSmaller`.
- **Execução:** quando não há job ativo e existe um pendente, `update` emite `Effect::StartJob(id)`. O `EffectRunner` cria um `Runner` com canal de eventos e `CancellationToken` próprios e repassa cada `JobEvent` como `Msg::Job(id, JobEvent)`. No fim, envia `Msg::JobFinished(id, Result<Report>)`.
- **Cancelamento:** `Effect::CancelJob(id)` dispara o token. O resultado chega como `Error::Cancelled` e vira `Cancelled`.
- **Sem paralelismo:** a ordem e a concorrência ficam triviais, deixando para `add-batch-queue` a generalização com N workers. A estrutura `Queue` já separa a política (o próximo pendente) da execução.

### D9. Terminal, sinais e saída
- **Inicialização:** `ratatui::init()` entra em modo raw e na tela alternativa e instala um panic hook que restaura o terminal. O retorno do `run` passa por um guard que chama `ratatui::restore()` em qualquer caminho, inclusive erro.
- **Ctrl+C:** em modo raw chega como tecla, não como SIGINT.
  - O primeiro Ctrl+C equivale a pedir para sair (com confirmação, se houver fila ativa).
  - O segundo Ctrl+C em até 2 s confirma a saída.
  - Um SIGTERM (via `tokio::signal::unix`) também confirma a saída.
- **Saída confirmada:** cancela o job ativo, aguarda o `JobFinished` com limite de 5 s, restaura o terminal e devolve 130 se algum job foi interrompido ou descartado da fila; caso contrário, 0.
- **Logs:** o `tracing` não pode escrever no terminal durante a TUI. Com `-v`, os logs vão para `$XDG_STATE_HOME/nago/tui.log`; sem `-v`, só erros vão para esse arquivo.

### D10. Navegador e teclado
- Lista ordenada com pastas primeiro, ordem natural, e oculta `.*` e `*.nago-*.part.*`.
- Filtro de extensões pela tabela de containers do core (`Container::parse` sobre a extensão) mais extensões de áudio comuns.
- Leitura de diretório e probes do arquivo destacado rodam como efeitos. O probe do arquivo destacado espera 150 ms de debounce para não disparar um ffprobe por linha ao rolar a lista.

**Atalhos principais** (todos listados em `?`; a fonte única é `nago-tui/src/keys.rs`). O passo de frame usa `<`/`>` porque `.` e `,` são digitados nos campos de tempo:

| Ação | Teclas |
|---|---|
| Navegar | `↑`/`↓`/`k`/`j` |
| Entrar na pasta | `Enter`/`l` |
| Pasta pai | `Backspace`/`h` |
| Marcar arquivo | `Espaço` |
| Mostrar todos os arquivos | `.` |
| Abrir operação | `c` cortar, `v` converter, `z` comprimir |
| Ver a fila | `Tab` |
| Sair | `q` |
| Corte: passo fino / médio / grosso | `<`/`>` (1 frame), `←`/`→` (1 s), `Shift+←`/`Shift+→` (10 s) |
| Corte: keyframe anterior / seguinte | `[`/`]` |
| Corte: alternar início/fim | `i`/`o` |
| Fila: cancelar job atual / remover selecionado | `x` / `d` |

## Risks / Trade-offs

- **[Risco] Seek lento para a miniatura em GOPs longos ou 4K** (decodificar até ~10 s de vídeo por frame) → debounce, cancelamento da extração obsoleta e cache LRU. A UI nunca espera a miniatura, e um indicador mostra "carregando".
- **[Risco] Terminal que não responde à consulta de protocolo** → a consulta do `ratatui-image` roda numa thread que lê o stdin até receber a resposta de status. Se a consulta expira, essa thread continuaria roubando teclas. Mitigação: ao falhar, a TUI envia um pedido de status (`ESC [ 5 n`) antes de o crossterm começar a ler, e a thread termina. Limite conhecido: com `TMUX` definido fora do tmux (variável herdada por engano), o terminal pode descartar esse pedido, e a primeira tecla se perde.
- **[Risco] Detecção de protocolo de imagem imprecisa** (tmux, SSH, terminais que respondem errado à consulta) → em caso de dúvida a miniatura fica desativada, que é o comportamento seguro da spec. A variável `NAGO_TUI_IMAGES=off|kitty|sixel|iterm2` força o protocolo.
- **[Risco] `prepare` faz probe e pode ser lento em arquivos de rede** → probe em cache por (caminho, mtime, tamanho). O formulário mostra "verificando…" sem bloquear.
- **[Trade-off] Reaplicar `prepare` antes de executar** pode mudar o nome de saída em relação ao mostrado → é o preço de nunca sobrescrever arquivos. O relatório deixa a mudança explícita.
- **[Trade-off] Fila sem persistência**: fechar a TUI perde os pendentes → a confirmação de saída informa quantos jobs serão descartados. A persistência fica para `add-batch-queue`.
- **[Risco] Dependência `ratatui-image` com API instável entre majors** → versão fixada no `Cargo.toml`, com o uso isolado num módulo `thumb.rs`.

## Migration Plan

Sem migração de dados. A única mudança visível na CLI é `nago` sem argumentos num TTY abrir a TUI em vez de mostrar a ajuda. Fora de TTY nada muda. Rollback: reverter o `Option<Command>` na CLI.
