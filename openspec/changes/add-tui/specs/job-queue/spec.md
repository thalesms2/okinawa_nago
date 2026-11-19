# Spec Delta

## Purpose

Executar na TUI uma fila de operações configuradas pelo usuário, uma após a outra, mostrando o andamento e o resultado de cada uma, com as mesmas garantias de execução da CLI (escrita atômica, cancelamento limpo, proteção de arquivos).

## ADDED Requirements

### Requirement: Execução sequencial
Cada formulário enviado SHALL gerar um ou mais jobs no fim da fila, um por arquivo selecionado. A fila SHALL executar um job por vez, na ordem de inserção, começando assim que houver um job pendente e nenhum em execução. O usuário MUST poder continuar navegando e enfileirando enquanto a fila roda.

#### Scenario: Enfileirar durante a execução
- **WHEN** uma compressão está em andamento e o usuário enfileira um corte
- **THEN** o corte fica pendente e começa automaticamente quando a compressão termina

### Requirement: Estados e progresso dos jobs
Cada job SHALL ter um estado visível entre: pendente, em execução, concluído, descartado (compressão que não reduziria o tamanho), falhou e cancelado. O job em execução SHALL mostrar a fase atual (por exemplo, codificando ou medindo VMAF), a porcentagem, a velocidade e o tempo restante estimado, atualizados pelo menos a cada segundo.

#### Scenario: Compressão com verificação
- **WHEN** um job de compressão com verificação de qualidade está rodando
- **THEN** a fila mostra a fase "codificando" com progresso e, depois, a fase "medindo VMAF" com seu próprio progresso

#### Scenario: Compressão que aumentaria o arquivo
- **WHEN** um job de compressão sem forçar produziria um arquivo maior que a entrada
- **THEN** o job termina no estado descartado, nenhum arquivo de saída permanece e a fila segue para o próximo job

### Requirement: Relatório por job
Ao terminar, cada job SHALL exibir o mesmo conteúdo do relatório final da CLI: caminho de saída, tamanho antes e depois, variação percentual, tempo gasto, encoder e perfil quando houver, ação por stream, VMAF médio e percentil 5 quando medidos, e avisos (por exemplo, meta de VMAF não atingida). Um job que falhou SHALL mostrar a mensagem de erro e o final do log do ffmpeg.

#### Scenario: Meta de VMAF não atingida
- **WHEN** um job de compressão termina com VMAF médio abaixo da meta do perfil
- **THEN** o relatório do job mostra o aviso com a meta e a sugestão de perfil, e a saída é mantida

#### Scenario: Falha do ffmpeg
- **WHEN** o ffmpeg falha durante um job
- **THEN** o job fica no estado falhou, mostra o erro e o fim do log, e a fila segue para o próximo job

### Requirement: Cancelamento e edição da fila
O usuário SHALL poder cancelar o job em execução e remover jobs pendentes. Cancelar um job MUST encerrar o ffmpeg e não deixar arquivo parcial, como no cancelamento da CLI, e a fila SHALL seguir para o próximo pendente. Jobs concluídos podem ser removidos da lista sem afetar seus arquivos.

#### Scenario: Cancelar o job atual
- **WHEN** o usuário cancela uma compressão em andamento com dois jobs pendentes
- **THEN** o ffmpeg é encerrado, nenhum arquivo parcial permanece, o job fica cancelado e o próximo pendente começa

#### Scenario: Remover pendente
- **WHEN** o usuário remove o segundo de três jobs pendentes
- **THEN** ele sai da fila e os outros dois rodam na ordem original

### Requirement: Saída com fila ativa
Ao pedir para sair com um job em execução ou pendente, a TUI SHALL pedir confirmação, informando quantos jobs serão interrompidos ou descartados. Confirmada a saída, o job em execução MUST ser cancelado sem deixar arquivo parcial antes de o terminal ser restaurado.

#### Scenario: Sair durante a compressão
- **WHEN** o usuário pede para sair com uma compressão em andamento e confirma
- **THEN** o ffmpeg é encerrado, nenhum arquivo parcial permanece, o terminal volta ao normal e o código de saída é 130

### Requirement: Códigos de saída da TUI
Sair interrompendo jobs SHALL resultar no código 130. Um SIGTERM recebido pelo processo, ou Ctrl+C pressionado duas vezes, SHALL ter o mesmo efeito que a saída confirmada. Saindo sem jobs interrompidos, o código SHALL ser 0, mesmo que algum job tenha falhado ou sido descartado.

#### Scenario: Saída com jobs que falharam
- **WHEN** a fila terminou com um job concluído e outro que falhou, e o usuário sai
- **THEN** o código de saída é 0

#### Scenario: SIGTERM com a TUI aberta
- **WHEN** o processo recebe SIGTERM durante um job
- **THEN** o job é cancelado sem deixar arquivo parcial, o terminal é restaurado e o código de saída é 130

### Requirement: Colisão de nomes entre jobs
Jobs com nome de saída automático que resultariam no mesmo caminho MUST receber nomes distintos, seguindo a regra de sufixo `-N` da CLI e considerando também as saídas dos jobs ainda pendentes. Um caminho de saída informado explicitamente no formulário que coincida com a saída de outro job da fila MUST ser recusado no envio. Nenhum job SHALL sobrescrever a saída de outro job nem um arquivo existente sem que o usuário tenha pedido sobrescrita explicitamente no formulário.

#### Scenario: Mesmo arquivo comprimido duas vezes
- **WHEN** o usuário enfileira duas compressões de `video.mp4` com perfis diferentes
- **THEN** as saídas são `video.compressed.mp4` e `video.compressed-1.mp4`

#### Scenario: Caminho explícito repetido
- **WHEN** um job pendente já vai gravar em `final.mkv` e o usuário envia outro formulário com saída `final.mkv`
- **THEN** o formulário informa que outro job da fila já usa esse caminho e não enfileira
