## \## Consulta obrigatória à documentação e ao Obsidian

## 

## \### 1. Fonte de documentação do projeto

## 

## O cofre oficial de documentação técnica do Ruana Studio está localizado em:

## 

## `F:\\Projetos Dev Temp\\ruana-studio\\ruana-studio-docs\\ruana-brain`

## 

## Esse cofre contém a documentação arquitetural, as decisões técnicas, os procedimentos de desenvolvimento, as informações sobre os módulos e o histórico de evolução do projeto.

## 

## Todos os agentes devem utilizá-lo como referência compartilhada durante o desenvolvimento.

## 

## Antes de implementar uma funcionalidade, corrigir um defeito, modificar uma API, refatorar um módulo ou alterar a integração entre componentes, consulte a documentação relacionada à tarefa.

## 

## Não dependa exclusivamente do contexto da conversa, de instruções recebidas do orquestrador ou do conhecimento geral do modelo.

## 

## \### 2. Procedimento obrigatório antes de programar

## 

## Para cada tarefa, o agente deve:

## 

## 1\. Ler o índice principal e as instruções gerais do cofre.

## 2\. Identificar os documentos relacionados à funcionalidade ou ao módulo afetado.

## 3\. Ler as decisões arquiteturais aplicáveis.

## 4\. Inspecionar os arquivos do código-fonte relacionados à tarefa.

## 5\. Verificar os manifests e as versões efetivamente utilizadas.

## 6\. Comparar a documentação com a implementação atual.

## 7\. Identificar divergências, limitações e dependências.

## 8\. Definir a estratégia de implementação antes de modificar código quando a complexidade justificar esse planejamento.

## 

## Se o cofre estiver inacessível, o agente deve informar a limitação e solicitar um meio de acesso ou as informações necessárias. Não deve afirmar que consultou documentos que não conseguiu ler.

## 

## Para correções pequenas e isoladas, a consulta pode ser proporcional ao escopo, mas as instruções gerais e as restrições relevantes continuam obrigatórias.

## 

## \### 3. Como encontrar informações no cofre

## 

## Não presuma nomes de arquivos, títulos de documentos ou uma estrutura de pastas que ainda não foi verificada.

## 

## Comece pelo índice existente e siga os links internos do Obsidian para encontrar a documentação relacionada.

## 

## Se houver uma ferramenta de pesquisa de arquivos disponível, utilize-a para localizar documentos por assunto. Se necessário, consulte o conteúdo integral dos documentos relevantes.

## 

## Pesquise por termos relacionados à tarefa, aos módulos envolvidos, às decisões arquiteturais e às limitações conhecidas.

## 

## Exemplos:

## \- Problemas de layout: documentação de Slint, componentes e temas.

## \- Problemas de renderização: documentação de wgpu, timeline, janela e integração gráfica.

## \- Problemas de reprodução: documentação de áudio, MIDI, transporte e sincronização.

## \- Problemas de persistência: documentação do projeto e do formato de arquivos.

## \- Problemas de compilação: documentação do workspace, dependências e procedimentos de build.

## 

## Esses exemplos são orientações de pesquisa, não uma declaração de que todos os documentos mencionados já existem.

## 

## \### 4. Documentação e código-fonte

## 

## A documentação do Obsidian define as decisões e os contratos arquiteturais registrados. O código-fonte e os manifests mostram a implementação efetiva.

## 

## Os agentes devem verificar ambos antes de tomar decisões.

## 

## Se a documentação afirmar que uma funcionalidade existe, confirme sua implementação antes de depender dela.

## 

## Se o código tiver mudado e a documentação estiver desatualizada, registre a divergência e determine qual informação precisa ser corrigida.

## 

## Se houver conflito envolvendo arquitetura, APIs públicas, dependências ou comportamento esperado, não escolha silenciosamente uma interpretação. Apresente as evidências e solicite uma decisão quando a mudança ultrapassar o escopo autorizado.

## 

## Não atualize a documentação para justificar retroativamente uma implementação incorreta.

## 

## \### 5. Uso do Obsidian como memória técnica compartilhada

## 

## O cofre deve funcionar como referência técnica persistente para todos os agentes do projeto.

## 

## Utilize-o para consultar:

## \- Arquitetura e responsabilidades dos módulos.

## \- Decisões arquiteturais e respectivas justificativas.

## \- APIs e contratos importantes.

## \- Dependências e versões.

## \- Procedimentos de build e testes.

## \- Problemas conhecidos e limitações.

## \- Histórico de alterações relevantes.

## \- Roadmap e tarefas planejadas.

## 

## Quando uma tarefa revelar informações importantes, registre-as no documento correspondente, seguindo a organização já existente.

## 

## Separe claramente:

## \- Fatos confirmados pela implementação.

## \- Decisões aprovadas.

## \- Hipóteses ainda não verificadas.

## \- Problemas conhecidos.

## \- Funcionalidades planejadas.

## \- Alterações concluídas e validadas.

## 

## Não registre uma hipótese como fato nem uma tarefa planejada como funcionalidade implementada.

## 

## \### 6. Investigação colaborativa entre os terminais 1, 2 e 3

## 

## Quando uma tarefa exigir investigação de maior complexidade, o orquestrador deve distribuir frentes complementares entre os agentes disponíveis nos terminais 1, 2 e 3.

## 

## Antes de iniciar a investigação, forneça a todos os participantes:

## \- O objetivo da tarefa.

## \- Os documentos relevantes do Obsidian.

## \- O contexto técnico já confirmado.

## \- As restrições arquiteturais.

## \- As perguntas específicas de cada frente.

## \- Os critérios de aceitação conhecidos.

## 

## Cada agente deve consultar a documentação pertinente e verificar suas conclusões no código-fonte.

## 

## A investigação pode ser dividida entre:

## \- Análise do fluxo de execução e dos módulos envolvidos.

## \- Verificação de hipóteses e alternativas técnicas.

## \- Reprodução do problema, testes e análise de regressões.

## 

## O orquestrador deve consolidar os resultados, identificar divergências e compartilhar as descobertas relevantes antes de decidir a implementação.

## 

## Não presuma que os terminais compartilham automaticamente o histórico ou conseguem ler as respostas uns dos outros. A comunicação deve utilizar os mecanismos efetivamente disponíveis no ambiente.

## 

## Para tarefas simples, o agente responsável pode consultar a documentação, investigar e implementar diretamente, sem exigir uma rodada desnecessária de colaboração.

## 

## \### 7. Uso da documentação durante a implementação

## 

## A consulta ao Obsidian não termina após o planejamento.

## 

## Durante a implementação, consulte novamente os documentos quando surgirem:

## \- Dependências não previstas.

## \- Dúvidas sobre contratos entre módulos.

## \- Divergências entre a arquitetura documentada e o código.

## \- Falhas de compilação que indiquem uma incompatibilidade.

## \- Necessidade de alterar interfaces compartilhadas.

## \- Decisões que possam afetar outras funcionalidades.

## 

## Antes de introduzir uma dependência, confirme as alternativas já utilizadas pelo projeto e verifique se existe uma decisão arquitetural relevante.

## 

## Antes de modificar um componente compartilhado, verifique seus consumidores e as restrições documentadas.

## 

## Antes de alterar o comportamento de áudio, MIDI ou renderização, consulte os documentos e testes relacionados para evitar regressões.

## 

## \### 8. Atualização da documentação após as alterações

## 

## Ao concluir uma tarefa, avalie se a documentação precisa ser atualizada.

## 

## Atualize os documentos pertinentes quando houver mudanças em:

## \- Arquitetura ou responsabilidades de módulos.

## \- APIs e contratos.

## \- Dependências ou versões.

## \- Procedimentos de configuração, build ou testes.

## \- Comportamento funcional relevante.

## \- Limitações conhecidas.

## \- Decisões arquiteturais.

## 

## Não crie documentos duplicados quando a informação puder ser acrescentada a um documento existente.

## 

## Registre a data da última verificação conforme o padrão do cofre e mantenha os links internos coerentes.

## 

## Se o código mudou, mas a documentação não precisou mudar, não invente atualizações artificiais.

## 

## \### 9. Relatório de conformidade documental

## 

## Ao concluir tarefas relevantes, informe:

## \- Quais documentos foram consultados.

## \- Quais arquivos do código foram verificados.

## \- Se a implementação estava alinhada à documentação.

## \- Quais divergências foram encontradas.

## \- Quais documentos foram atualizados.

## \- Quais dúvidas continuam pendentes.

## 

## Os relatórios devem distinguir documentos efetivamente lidos daqueles apenas identificados pelo nome.

## 

## \### 10. Regra de prioridade

## 

## Nenhum agente deve iniciar uma alteração relevante ignorando deliberadamente a documentação disponível.

## 

## O fluxo de trabalho deve combinar três elementos:

## 

## 1\. \*\*Obsidian:\*\* decisões arquiteturais, conhecimento técnico e histórico compartilhado.

## 2\. \*\*Código-fonte:\*\* implementação efetiva, contratos existentes e comportamento verificável.

## 3\. \*\*Testes:\*\* evidências de funcionamento e proteção contra regressões.

## 

## O agente deve consultar a documentação, verificar a implementação e validar as alterações. Quando esses elementos divergirem, deve investigar a causa e registrar a decisão tomada.

## 

## O objetivo é garantir que todos os agentes trabalhem com o mesmo conhecimento técnico, sem inventar funcionalidades, repetir investigações desnecessárias ou introduzir mudanças incompatíveis com o Ruana Studio.

