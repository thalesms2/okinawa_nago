# job-execution Specification

## Purpose

Executar as operações de mídia de forma segura e observável: progresso em tempo real, cancelamento limpo, saída atômica que nunca corrompe nem sobrescreve arquivos indevidamente, e relatório final legível por humanos e scripts.

## Requirements

### Requirement: Progresso em tempo real
Durante operações que processam mídia, quando stderr for um terminal, o sistema SHALL exibir percentual concluído, velocidade relativa (ex.: `3.2x`) e tempo restante estimado, atualizados ao menos uma vez por segundo.

#### Scenario: Execução interativa
- **WHEN** o usuário comprime um vídeo de 10 minutos em um terminal
- **THEN** uma barra de progresso mostra percentual, velocidade e ETA até a conclusão

#### Scenario: Execução não interativa
- **WHEN** a saída de erro é redirecionada para um arquivo
- **THEN** nenhuma barra de progresso animada é escrita

### Requirement: Escrita atômica da saída
O sistema MUST gravar a saída em um arquivo temporário no diretório de destino e renomeá-lo para o nome final somente após o sucesso, de modo que o nome final nunca contenha um arquivo parcial.

#### Scenario: Falha no meio do processamento
- **WHEN** o ffmpeg falha com 50% concluído
- **THEN** não existe arquivo com o nome final nem arquivo temporário residual

### Requirement: Cancelamento limpo
Ao receber SIGINT ou SIGTERM, o sistema SHALL encerrar o processo ffmpeg em andamento, remover arquivos temporários e sair com código 130.

#### Scenario: Ctrl+C durante a compressão
- **WHEN** o usuário pressiona Ctrl+C durante uma compressão
- **THEN** o ffmpeg é encerrado, nenhum arquivo parcial permanece e o código de saída é 130

### Requirement: Proteção de arquivos existentes
O sistema MUST NOT sobrescrever o arquivo de entrada em nenhuma hipótese. Para nomes de saída padrão, se o nome já existir, o sistema SHALL acrescentar um sufixo numérico (`-1`, `-2`, ...). Para `-o` explícito apontando para arquivo existente, o sistema MUST falhar, a menos que `--overwrite` seja informado.

#### Scenario: Saída igual à entrada
- **WHEN** o usuário executa `nago compress a.mp4 -o a.mp4 --overwrite`
- **THEN** o sistema recusa a operação informando que a entrada não pode ser sobrescrita e sai com código 2

#### Scenario: Nome padrão já existente
- **WHEN** `aula.cut.mp4` já existe e o usuário corta `aula.mp4` sem `-o`
- **THEN** a saída é gravada como `aula.cut-1.mp4`

#### Scenario: Saída explícita existente
- **WHEN** o usuário informa `-o final.mp4`, `final.mp4` existe e `--overwrite` não foi informado
- **THEN** o sistema informa o conflito e sai com código 2 sem processar

### Requirement: Relatório final
Ao concluir com sucesso, o sistema SHALL informar o caminho da saída, os tamanhos de entrada e saída, a variação percentual de tamanho, o tempo decorrido e a operação aplicada (streams copiados/recodificados, encoder, perfil). Com `--json`, esse relatório SHALL ser o único conteúdo de stdout, como um objeto JSON.

#### Scenario: Relatório humano
- **WHEN** uma compressão reduz um arquivo de 1,2 GB para 380 MB
- **THEN** o relatório mostra `1.2 GB → 380 MB (-68%)`, o tempo decorrido e o encoder usado

#### Scenario: Relatório JSON
- **WHEN** o usuário adiciona `--json`
- **THEN** stdout contém um objeto JSON com `output`, `input_size_bytes`, `output_size_bytes`, `elapsed_secs` e `encoder`

### Requirement: Erros do ffmpeg legíveis
Quando o ffmpeg falhar, o sistema SHALL exibir uma mensagem resumida e as últimas linhas relevantes do log do ffmpeg, e SHALL salvar o log completo quando `--verbose` for informado.

#### Scenario: Falha de codificação
- **WHEN** o ffmpeg termina com erro
- **THEN** o sistema mostra o erro resumido e as últimas linhas do log e sai com código 1

### Requirement: Códigos de saída
O sistema SHALL usar códigos de saída consistentes: 0 para sucesso, 1 para erro de execução, 2 para erro de uso ou validação, 3 para operação descartada por não reduzir o tamanho e 130 para cancelamento.

#### Scenario: Argumento inválido
- **WHEN** o usuário informa uma opção desconhecida
- **THEN** o sistema mostra a ajuda resumida e sai com código 2
