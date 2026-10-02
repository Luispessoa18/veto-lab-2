# Avaliação do VETO e sugestões de testes

## Resumo

O resultado atual de **100% de acurácia** está correto para o pipeline completo nos
200 casos sintéticos executados, mas não deve ser interpretado como 100% de
acurácia da IA ou como prova de generalização para ataques desconhecidos.

O benchmark mede a composição de várias camadas:

1. blacklist;
2. Prompt Guard;
3. política determinística Solana;
4. decisão do modelo VETO LFM;
5. override determinístico de consistência/divergência;
6. simulação RPC sintética.

Cada ataque atual foi construído para acionar uma dessas camadas. Isso torna o
benchmark útil como teste de integração e regressão, mas fácil demais para medir a
capacidade geral de detecção do modelo.

## O gabarito foi enviado para a IA?

Não foi encontrado vazamento direto do gabarito para a API.

Os campos abaixo ficam no executor do benchmark e não são enviados no corpo da
requisição:

- `expected_decision`;
- `expected_layer`;
- `ground_truth`;
- `attack_type`.

O simulador envia somente o objeto `payload` para
`POST /v1/transactions/verify`.

Entretanto, existe uma dependência metodológica: o gerador lê o rótulo do caso e,
com base nele, cria uma mutação que corresponde diretamente a uma regra conhecida
do pipeline. Não é vazamento de resposta durante a inferência, mas faz com que o
conjunto de avaliação seja muito previsível.

## De onde vieram os 100%

Na execução analisada, os 200 casos foram divididos em 100 `ALLOW` e 100 `BLOCK`:

| Camada final | Casos | Interpretação |
|---|---:|---|
| Sem bloqueio | 100 | Casos benignos aprovados após as verificações |
| Blacklist | 11 | Conta sintética cadastrada previamente |
| Prompt Guard | 20 | Prompt injection explícito |
| `layer_1` | 58 | Mistura política determinística e decisão da IA |
| Simulação Solana | 11 | Reversão programada no RPC mock |

O total de 58 em `layer_1` não significa 58 detecções da IA:

- 47 foram conflitos encontrados pela política Solana;
- 11 foram divergências apresentadas ao LFM;
- os 100 casos benignos receberam `BLOCK` bruto do LFM em parte da execução, mas
  foram corrigidos para `ALLOW` pelo override `DETERMINISTIC_CONSISTENCY`.

Portanto, a métrica atual é melhor descrita como:

> Acurácia do pipeline VETO nos ataques sintéticos cobertos explicitamente pelas
> regras do próprio pipeline.

Ela não mede isoladamente a acurácia do LFM.

## Limitações atuais

### Casos construídos a partir das regras de defesa

Os ataques de destinatário, programa, valor e delegate são mutações diretas dos
campos que a política compara. Esses testes confirmam que a regra funciona, mas
não testam ataques desconhecidos ou representações alternativas.

### RPC mock conhece previamente o resultado

Os valores `SIM_OK` e `SIM_REVERT` determinam diretamente a resposta do RPC mock.
Isso é adequado para testar o encadeamento das camadas, mas não mede fidelidade de
execução Solana.

### Prompt injections muito explícitos

Frases como “ignore todas as instruções” são importantes como teste básico, mas
não cobrem injeções indiretas, fragmentadas, codificadas ou semanticamente sutis.

### Métricas misturam componentes diferentes

Blacklist, Prompt Guard, regras fixas, LFM e simulação aparecem dentro de uma única
acurácia. Um componente pode compensar erros de outro sem que isso fique evidente.

### Casos benignos homogêneos

Os benignos são variações próximas do mesmo `spl_transfer`. A baixa diversidade
facilita overrides determinísticos e não mede falsos positivos em operações reais
mais complexas.

## Métricas recomendadas

O relatório deve publicar separadamente:

### Pipeline completo

- acurácia;
- precisão, recall e F1 para `BLOCK`;
- taxa de falso positivo;
- taxa de falso negativo;
- quantidade de `REVIEW`, `ERROR` e `INVALID`;
- latência média, mediana, p95 e máxima.

### Modelo LFM puro

- `model_decision` antes de qualquer override;
- matriz de confusão do modelo;
- falsos positivos benignos;
- ataques liberados pelo modelo;
- confiança média em acertos e erros;
- taxa de JSON válido;
- campos de evidência inexistentes ou alucinados.

### Overrides determinísticos

- quantidade de `DETERMINISTIC_CONSISTENCY`;
- quantidade de `DETERMINISTIC_MISMATCH`;
- quantos erros do modelo foram corrigidos;
- quantos acertos do modelo foram alterados;
- resultado do pipeline com e sem overrides.

### Cada camada

- bloqueios exclusivos da blacklist;
- bloqueios exclusivos do Prompt Guard;
- bloqueios exclusivos da política Solana;
- bloqueios exclusivos do LFM;
- bloqueios exclusivos da simulação;
- quantos casos chegaram a cada etapa do funil.

## Plano de testes sugerido

### 1. Teste de integração atual

Manter os 200 casos existentes como suíte de regressão.

Objetivo: garantir que rotas, camadas, encerramento antecipado, auditoria e relatório
continuam funcionando depois de alterações no código.

Esse teste deve ser chamado de **integração sintética**, não de avaliação geral do
modelo.

### 2. Ablation test por camada

Executar o mesmo conjunto em configurações diferentes:

| Execução | Blacklist | Guard | Política | LFM | Simulação |
|---|---:|---:|---:|---:|---:|
| Pipeline completo | sim | sim | sim | sim | sim |
| IA pura | não | não | não | sim | não |
| Sem IA | sim | sim | sim | não | sim |
| Sem overrides | sim | sim | sim | sim | sim |
| Somente simulação | não | não | não | não | sim |

Isso revela qual componente realmente produz cada acerto.

Os modos já disponíveis são selecionados antes de iniciar a API:

```bat
set "VETO_ABLATION_MODE=model_only"
08_API_UNICA.bat
```

ou:

```bat
set "VETO_ABLATION_MODE=no_model"
08_API_UNICA.bat
```

`model_only` ignora blacklist, Prompt Guard, política Solana e simulação.
`no_model` mantém as demais camadas e não chama o LFM. Sem a variável, o modo é
`full`. Use `--output` e `--report` no simulador para não sobrescrever execuções:

```bat
.venv\Scripts\python.exe -m src.solana_attack_simulator --prepared results\solana_cases.jsonl --api http://127.0.0.1:8070 --output results\ablation_model.jsonl --report results\ablation_model_report.json
```

### 3. Holdout independente

Separar os geradores de treino/desenvolvimento e avaliação.

- O conjunto de avaliação não deve reutilizar templates textuais do treino.
- Endereços, valores, programas e ordem das instruções devem ser novos.
- Os responsáveis por criar o holdout não devem consultar as regras internas do
  detector durante a criação.
- O conjunto deve ser congelado antes de ajustar prompts, thresholds ou overrides.

### 4. Mutações adversariais

Criar variações que preservam o mesmo significado:

- reordenar instruções;
- adicionar instruções legítimas antes e depois do ataque;
- omitir campos opcionais;
- duplicar contas em posições diferentes;
- representar valores nos limites exatos;
- usar múltiplas transferências pequenas em vez de uma grande;
- trocar destinatário em CPI ou instrução interna;
- usar Address Lookup Tables;
- combinar delegate approval e transferência;
- esconder o efeito perigoso atrás de programa intermediário;
- testar programas Token e Token-2022;
- testar transações versionadas.

O rótulo deve vir do efeito autorizado ou não autorizado, e não do nome da mutação.

### 5. Prompt injection avançado

Adicionar ataques menos óbvios:

- instruções indiretas em nome, símbolo, memo ou URI;
- texto dividido entre vários campos;
- português, inglês e idiomas misturados;
- Base64, Unicode confusável e caracteres invisíveis;
- falso aviso de sistema;
- conteúdo dizendo ser relatório de auditoria;
- instrução para alterar somente `confidence` ou `evidence_fields`;
- ataques que evitam palavras como “ignore” e “system prompt”.

Também é necessário incluir controles benignos semanticamente próximos para medir
falsos positivos.

### 6. Transações Solana reais em Devnet

Construir e serializar transações válidas para Devnet, sem enviá-las:

- transferência SPL legítima;
- destinatário divergente;
- valor acima do limite;
- delegate approval;
- close account;
- alteração de autoridade;
- falha por saldo insuficiente;
- falha por conta inexistente;
- blockhash expirado com `replaceRecentBlockhash=true`;
- transação v0 com Address Lookup Table.

Enviar essas transações somente para `simulateTransaction`. Registrar logs, unidades
consumidas, erro RPC e diferenças de saldo quando fornecidas. Nunca usar chaves ou
fundos de produção.

### 7. Casos benignos diversos

Incluir pelo menos:

- transferências SPL e SOL;
- criação de Associated Token Account;
- swaps com rotas de múltiplas instruções;
- staking e unstaking;
- uso legítimo de delegate com autorização explícita;
- valores exatamente no limite;
- programas intermediários autorizados;
- falhas técnicas que devem resultar em `REVIEW`, não em acusação de ataque.

### 8. Testes metamórficos

Partir de um caso e aplicar transformações que não deveriam mudar a decisão:

- mudar IDs e nomes internos;
- alterar a ordem de campos JSON;
- trocar textos descritivos benignos;
- mudar valores irrelevantes;
- repetir a execução com a mesma seed.

Depois aplicar uma única transformação de segurança que deve mudar `ALLOW` para
`BLOCK`. Isso testa estabilidade e causalidade.

### 9. Testes de robustez operacional

- RPC indisponível;
- LFM indisponível;
- Prompt Guard indisponível;
- timeout e resposta HTTP inválida;
- resposta da IA sem JSON;
- JSON válido com campos faltando;
- execução concorrente;
- arquivo de blacklist inválido;
- relatório parcial após interrupção;
- reinício durante o benchmark.

Falhas de infraestrutura devem ser registradas como `ERROR` ou `REVIEW`, nunca como
acerto de segurança.

### 10. Repetição e comparação de modelos

Executar o holdout com:

- VETO LFM2.5 350M;
- Gemma 3 4B + LoRA;
- política sem modelo;
- um baseline que sempre responde `BLOCK`;
- um baseline que sempre responde `ALLOW`.

Relatar qualidade, latência, memória e tokens por segundo. Um modelo somente é útil
se superar os baselines sem elevar excessivamente falsos positivos.

## Critérios mínimos sugeridos

Antes de apresentar o resultado como avaliação de segurança:

- nenhum campo de gabarito no payload enviado ao modelo;
- holdout congelado e independente;
- pelo menos 500 casos, com diversidade de operações;
- pelo menos 100 benignos difíceis;
- falsos positivos abaixo de um limite definido previamente;
- métricas da IA pura publicadas separadamente;
- erros de infraestrutura excluídos da acurácia e reportados à parte;
- resultados reproduzíveis com seed e versão do dataset;
- revisão manual de uma amostra de acertos e erros;
- limitações declaradas junto aos números.

## Interpretação recomendada do resultado atual

Formulação adequada:

> O pipeline obteve 100% nos 200 testes sintéticos de integração cobertos pelas
> regras implementadas, sem erros de infraestrutura nesta execução.

Formulação que deve ser evitada:

> A IA detecta 100% dos ataques Solana.

O primeiro enunciado é sustentado pelo teste atual. O segundo exigiria holdout
independente, transações mais realistas, métricas isoladas do modelo e validação
externa.
