# VETO LAB Windows — benchmark de carteira / Gemma GGUF

**Estado honesto desta versão**: executável para 240 casos **sintéticos pareados** com simulação DETERMINÍSTICA EM PYTHON (não Anvil), catalogação de arquivos reais DeFiHackLabs em múltiplas redes e consulta opcional read-only a hashes encontrados. Reproduções `forge test` opcionais são mantidas **em separado**, sem se passar por resultados Anvil ou por rótulos de transações. Não existe integração automática que converta cada ataque histórico em uma transação pré-ataque + pós-execução validada. Não use métricas sintéticas como taxa de detecção de hacks reais.

## Requisitos
- Windows 10/11 x64; Python 3.11+; Git for Windows (para baixar DeFiHackLabs).
- GGUF VETO Security exportado no Colab: copie para `models\`.
- llama.cpp **Windows pré-compilado** com `llama-server.exe`: baixe a variante **CPU** ou **CUDA** apropriada em https://github.com/ggml-org/llama.cpp/releases . Extraia o executável **e todas as DLLs e demais arquivos do pacote** na pasta `llama\`; apenas copiar o EXE pode falhar. Não distribuí binários terceiros no ZIP.
- GPU Nvidia: escolha release CUDA compatível e drivers atualizados. `02_SERVIDOR.bat` usa `-ngl 99`; se a GPU não suportar, altere para `-ngl 0` (CPU).
- Para `forge test` opcional, instale Foundry (`forge`) no PATH. Para fork histórico são necessários RPCs de arquivo / blocos correspondentes e dependências do repositório; essas execuções podem falhar.

## Uso rápido (CMD/bat, sem PowerShell)
1. Execute `01_INSTALAR.bat`.
2. Coloque o seu arquivo `*.gguf` em `models\`; extraia llama.cpp em `llama\`.
3. Abra `02_SERVIDOR.bat`; espere `/health` responder OK. O servidor permanece aberto.
4. Abra `07_PROMPT_GUARD.bat` em outra janela; espere o endpoint `http://127.0.0.1:8090/health`. Ele usa o Meta Llama Prompt Guard 2 86M localmente em CPU.
5. Em outra janela, execute `03_GERAR_CASOS.bat`. Gera 240 cenários sintéticos balanceados e clona/cataloga até 80 arquivos do repo real por rede identificada.
6. Execute `04_RODAR_BENCHMARK.bat`. Toda metadata textual não confiável passa primeiro pelo Prompt Guard; somente entradas benignas chegam ao Gemma em `http://127.0.0.1:18080/v1/chat/completions`.
7. Leia `results\metrics.json`, `results\predictions.jsonl` e `results\prompt_guard.jsonl`. Execute `05_RELATORIO.bat` para recalcular métricas detalhadas e contagens por tipo.

## API única de decisão

Para iniciar llama.cpp, Anvil, Prompt Guard e API de uma vez, execute `00_INICIAR_TUDO.bat`. O navegador abrirá o Swagger em `http://127.0.0.1:8070/docs`. A especificação OpenAPI também fica disponível em `/openapi.json`. O inicializador procura o Anvil no `PATH` e em `%USERPROFILE%\.foundry\bin\anvil.exe`.

Como alternativa, com o llama.cpp (`02_SERVIDOR.bat`) e o Anvil na porta 8545 ativos, execute `08_API_UNICA.bat`. A única API pública fica em `http://127.0.0.1:8070` e grava auditoria em `results/api_audit.jsonl`.

- `POST /v1/transactions/verify`: executa blacklist de wallets, camada 1 (política + IA) e, somente se aprovado, simulação Anvil. Um `BLOCK` ou `REVIEW` interrompe as etapas seguintes.
- `POST /v1/agent/run`: verifica o campo `prompt` com o Prompt Guard antes de chamar a IA. Injection retorna `decision=BLOCK`, `security_alert=true` e `blocked_by=prompt_guard`.
- `GET /health`: saúde e rotas disponíveis.

### Benchmark de ataques Solana

Com a API ativa, execute `09_SIMULAR_ATAQUES_SOLANA.bat`. Ele consome os 200 fixtures de `synthetic_cases.jsonl`, prepara versões Solana balanceadas e testa blacklist, substituição de destinatário e programa, excesso de valor, delegate approval, divergência detectável pela IA, falha de simulação e prompt injection. Todos os casos entram pela rota pública e respeitam o encerramento antecipado das camadas. O RPC usado pelo benchmark é local e sintético: nenhuma transação é enviada à Solana. Os eventos completos ficam em `results/solana_attack_simulation.jsonl` e o funil por camada em `results/solana_attack_report.json`.

Cadastre endereços em `config/blacklist.json`. O gerador acrescenta contas Solana sintéticas e determinísticas para exercitar a blacklist; elas não representam uma acusação contra contas reais. Para o Anvil, envie uma transação RPC em `anvil.params` ou os campos `from`, `to`, `value`, `data`, `gas` e `gasPrice` em `transaction`. A resposta inclui `layers`, `blocked` e `blocked_by`, deixando claro quais camadas chegaram a executar.

Para fazer uma rodada rápida de 4 cenários: `.venv\Scripts\python.exe -m src.main run --limit 4`.
Para gerar 300 cenários: `.venv\Scripts\python.exe -m src.main generate --count 300`.

## O que cada fonte realmente é
- `results/synthetic_cases.jsonl`: cenários criados localmente, com autorização, tx proposta e efeitos calculados em Python; não existe swap de token real nem estado on-chain real. A variação legítima inclui taxa incluída no montante autorizado; a adversarial muda recipient, taxa, spender/allowance ou alvo. Pares recebem a mesma base e mudam somente um vetor relevante.
- Os sintéticos cobrem 10 padrões legítimos difíceis (incluindo fee wallet autorizada, intermediário verificado e allowance no limite) e 10 tipos de ataque. Ataques sem evidência disponível, como router autorizado comprometido, são marcados como não observáveis e aparecem separados nas métricas; não devem ser tratados como falha detectável do agente.
- O conjunto atual também inclui prompt injections em inglês e português e controles benignos semanticamente próximos. O Prompt Guard é uma camada adicional, não uma garantia absoluta; decisões e scores ficam auditados em `results/prompt_guard.jsonl`.
- `results/historical_catalog.jsonl`: **código-fonte REAL do DeFiHackLabs**, indicação de rede inferida, URL do arquivo e hashes candidatos. Um hash presente num arquivo não prova ser a transação exploradora, e a rede inferida pode estar errada. Todos os casos catalogados têm `ground_truth.label = UNVERIFIED` e não entram no relatório de precisão.
- `results/predictions.jsonl`: decisões e respostas antes/depois dos casos sintéticos. Os rótulos só entram em avaliação **depois** das chamadas ao modelo. A saída pode conter informações de teste disponíveis no estágio, nunca a propriedade `ground_truth`.
- `results/metrics.json`: TP, FN, FP, TN; REVIEW e INVALID/ERROR em categorias separadas. `ALLOW` para um ataque conta como ataque perdido; `BLOCK` para legítima conta como falso positivo. Resultados são de cenários sintéticos, não de incidentes reais.

## Coleta real opcional (não deve ser confundida com simulação)
Edite `config/settings.json` para preencher `rpc_by_chain` com endpoints RPC **de leitura**, depois execute `06_CATALOGO_HISTORICO.bat`. O programa busca até dois hashes candidatos por arquivo e guarda `eth_getTransactionByHash` e `eth_getTransactionReceipt` quando disponíveis. `logs` da receipt não são convertidos automaticamente em deltas de saldo, transferência econômica comprovada ou intenção do usuário; não se infere rótulo.

Opcional: ` .venv\Scripts\python.exe -m src.main catalog --forge --max-replays 3 ` executa apenas os **testes Foundry originais** correspondentes. Instale dependências upstream e forneça RPCs de fork de cada rede. `TEST_PASSED` **não** significa que o VETO detectou o ataque ou que ele foi reproduzido no Anvil: o Foundry usa seu próprio EVM/fork e o projeto ainda não transforma traces em alterações patrimoniais verificadas. Os logs permanecem em `historical_catalog.jsonl` e não são enviados como se fossem avaliações pré-assinatura.

## Para adicionar simulação real Anvil de todos os casos
A próxima integração deve produzir, caso a caso: (1) bloco/estado anterior verificáveis, (2) transação original ou cenário de agente + autorização confiável, (3) `eth_call`/`debug_traceCall` preventivos ou replay Foundry sobre snapshot sem vazamento de futuro, (4) execução no fork isolado e deltas líquidos token/native, allowances/proxy, (5) prova e revisão humana do gabarito, (6) pares negativos concretos da mesma rede/contrato. Não existe forma fiel de inferir o pedido do usuário a partir do histórico on-chain. Replays de exploits podem depender de mempool, oráculos e ordem de transações; `eth_call` no estado atual não recompõe automaticamente o bloco histórico.

## Precauções
- O servidor fica restrito a 127.0.0.1; não exponha sua carteira nem private keys reais. Nunca execute esses testes na mainnet.
- Os pares sintéticos fornecem intenção explícita, mas históricos têm intenção desconhecida. Não atribua `ALLOW`/`BLOCK` a histórico com base apenas no nome do arquivo.
- O script de coleta `git clone` acessa repositório público no momento da execução e os casos mudam ao longo do tempo. Registre commit para benchmark reproduzível em publicação.
