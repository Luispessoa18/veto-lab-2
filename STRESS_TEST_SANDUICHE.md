# Stress test de sanduíche: proposta para revisão

*Proposta · 2026-10-07 · autor: Lucas Oliveira · revisor: Luis*
*Status: **para revisão**. Nada aqui está implementado ainda.*
*Depende de: PR #1 (aval-svm). Ordem dos PRs: #1 → #2 → este.*

> **Como revisar:** comente direto nas linhas do PR. As decisões abertas estão no fim (seção 9). Cada uma tem uma recomendação; basta responder "ok" ou propor outra.

---

## 1. Por que isso importa

Um **ataque de sanduíche** é o caso mais claro da tese do VETO: *uma transação que não quebra nenhuma regra, mas não é o que o usuário pediu.*

O agente pede "comprar token X". A transação é autorizada, o valor está abaixo do teto e o programa está na allowlist, então todas as políticas passam. Mesmo assim, um bot compra **antes**, a vítima compra **mais caro**, e o bot vende **depois**. Os três vão num bundle atômico da Jito. O lucro do bot sai do bolso da vítima, na forma de slippage.

Exemplo real (slides da Helius):

| Ordem | Quem | Operação | Valores |
|---|---|---|---|
| 1 | MEV Bot | BUY na Raydium | 14,63 SOL → 32,94 mi Komeko |
| 2 | Vítima | BUY na Raydium | 0,33 SOL → 624 mil Komeko |
| 3 | MEV Bot | SELL na Raydium | 32,9 mi Komeko → 14,65 SOL |

Lucro bruto do bot: **0,02 SOL + 4 mil Komeko**.

**Por que o agente está exposto:**

- Na Solana, transações normais passam pelo Jito Relayer, que as **segura por ~200 ms** antes de chegarem ao líder.
- Bundles de searchers chegam pelo Block Engine e são executados de forma atômica.
- Um agente que manda swaps com tolerância de slippage folgada é um alvo previsível.

**Escopo:** não nos importam a arbitragem e o backrun, que não prejudicam diretamente o usuário. O foco é o **sanduíche**.

## 2. O que o Aval consegue fazer que ninguém mais consegue

O aval-svm (PR #1) roda uma **VM Solana nossa**, com o estado real do cluster. Um RPC público só simula a transação que você manda, sozinha. Nós podemos simular **o mundo em volta dela**:

```
estado real ──► [front-run sintético do atacante] ──► [tx do agente] ──► [back-run sintético] ──► medir
```

Isso responde, **antes da assinatura**: *"se um bot sanduichar esta transação, ela ainda passa? quanto o usuário perde? quanto o bot ganha?"*

É determinístico, local e sem IA. O tempo-alvo é da ordem de milissegundos por candidato.

## 3. O que vamos medir

Para cada transação de swap do agente:

| Medida | Definição | De onde vem |
|---|---|---|
| `expected_out` | quanto o agente recebe **sem** ataque | simulação normal (já existe: `/v1/project`) |
| `min_out` | o mínimo que a transação aceita | dados da instrução de swap (por DEX) **ou** busca binária do stress test (ver §4.3) |
| `extractable` | `expected_out − min_out`: o **máximo** que um sanduíche pode tirar | derivado |
| `worst_case_out` | quanto o agente recebe sob o **pior** sanduíche que ainda deixa a tx passar | stress test |
| `attacker_profit` | lucro do atacante nesse pior caso (descontadas as taxas) | stress test |
| `exposure_bps` | `(expected_out − worst_case_out) / expected_out`, em pontos-base | derivado |
| `sandwichable` | `attacker_profit > custo mínimo de um bundle` (gorjeta Jito + taxas) | derivado |

A **saída** é um finding novo, `mev.sandwich_exposure`, com essas medidas e uma recomendação concreta (§5).

## 4. Como o stress test funciona (aval-svm)

### 4.1 Encontrar a pool

1. Simular a transação do agente normalmente (o que já fazemos).
2. Entre as contas graváveis, identificar as que pertencem a programas de AMM conhecidos.
3. Pelo diff de saldos (a projeção que já existe), confirmar **qual token entra e qual sai**. Isso dá a direção do swap.

### 4.2 Montar o atacante (só dentro da VM)

1. Criar **dentro da VM** uma carteira sintética de atacante, com SOL e contas de token já carregadas (`set_account`). Não existe chave real e não existe dinheiro real.
2. **Front-run:** clonar a instrução de swap do próprio agente, trocando as contas do usuário pelas do atacante, **na mesma direção**, com tamanho `x`.
3. **Back-run:** a operação inversa, vendendo tudo o que o atacante recebeu.

Reaproveitar a instrução da vítima evita reimplementar a matemática de cada AMM.

### 4.3 Executar e buscar o pior caso

Precisa de um modo "**executar e commitar**" num **snapshot descartável** da VM. A simulação atual não commita estado. Cada candidato roda numa cópia limpa.

```
para cada tamanho x (busca binária entre 0 e o tamanho máximo útil):
    snapshot ← estado real
    executar front-run(x)
    executar tx do agente   → falhou? então x é grande demais (o min_out da vítima segurou)
    executar back-run
    medir: saída do agente, lucro do atacante
devolver o x que maximiza o lucro do atacante com a tx do agente ainda passando
```

- Uns **15–20 candidatos** bastam.
- Sem `min_out` decodificado, a própria busca acha o limite: é o maior `x` com que a tx da vítima ainda passa.

### 4.4 Interface

```
POST /v1/stress   { "transaction": "<base64>", "budget_ms": 50 }
→ { "sandwichable": true, "exposure_bps": 312, "extractable": {...}, "worst_case_out": "...",
    "attacker_profit_lamports": "...", "front_run_size": "...", "pools": ["..."],
    "recommendation": { "min_out": "...", "use_jito_dont_front": true }, "aval": { "digest": "...", ... } }

aval-svm stress --tx <base64>            (CLI, mesma saída)
```

Se a DEX não for suportada, ou o tempo acabar, a resposta é `{"supported": false, "reason": "..."}`. **Nunca** um "seguro" falso.

### 4.5 DEXs na v1

| DEX | v1? | Motivo |
|---|---|---|
| Raydium AMM v4 | **sim** | onde estão os sanduíches dos exemplos; instrução direta |
| Orca Whirlpool | **sim** | CLMM muito usada; instrução direta |
| Raydium CLMM | depois | |
| Jupiter (rotas) | depois | rota multi-hop; o front-run precisa mirar a pool mais rasa da rota |
| pump.fun / bonding curves | depois | muito alvo de sanduíche; curva própria |

## 5. O que o VETO faz com isso

O **gate não altera a transação do agente**: ele permite, sinaliza ou bloqueia. Ele **recomenda** a correção.

| Situação | Veredito | Recomendação devolvida |
|---|---|---|
| `sandwichable = false` | allow | — |
| exposição baixa (< limite configurável, ex. 50 bps) | flag | apertar `min_out` para `expected_out × (1 − tolerância realista)` |
| exposição alta | deny | apertar `min_out` **e** enviar com proteção anti-front-run da Jito (conta `jitodontfront…` na tx; confirmar a regra exata na doc da Jito) **e/ou** asserção on-chain (Lighthouse, item **H1** do backlog do VETO) |
| DEX não suportada | flag `coverage.mev_unknown` | — |

**Atestação (PR #2):** dá para gravar o resultado do stress test junto do veredito. Prova on-chain de que *"o Aval viu o risco e bloqueou"*. Isso é material forte para o pitch.

## 6. Dados, avaliação e treino

O lab já tem a cultura certa (veja `AVALIACAO_E_TESTES.md`): casos pareados, holdout, métricas separadas e honestidade sobre o que é sintético. A proposta encaixa o sanduíche nisso.

### 6.1 Rótulos vêm da simulação, não do modelo

O stress test é **determinístico**, então ele **gera o gabarito**. O modelo (LFM/LoRA do lab) **não** decide se há exposição. O papel dele é **explicar**, em linguagem do usuário, e **priorizar**. É o mesmo princípio do VETO: *o modelo pontua, a regra decide*.

### 6.2 Três conjuntos de dados

| Conjunto | Origem | Para quê |
|---|---|---|
| **A. Swaps reais** | transações de swap em blocos reais da mainnet (o `aval-svm shadow` já sabe amostrar blocos), passadas pelo stress test | distribuição real de exposição: quantos swaps são sanduicháveis, e com quanto |
| **B. Sanduíches reais (ground truth)** | detectar no histórico o padrão **A-compra → V-compra → A-vende** na mesma pool, no mesmo slot ou bundle | **validação**: o stress test, rodado no estado *anterior* à tx da vítima, teria dito "sanduichável"? com que exposição? |
| **C. Cenários de agente sintéticos** | gerador do lab (`03_GERAR_CASOS`) com dois tipos novos: `sandwich_exposure` (slippage folgado, ataque) e `benign_tight_slippage` (mesmo swap, slippage justo, benigno), **pareados** | treino e benchmark da camada de IA e do pipeline inteiro |

O conjunto **B** é o mais importante: é a prova de que o stress test acerta o mundo real, não só os nossos sintéticos.

### 6.3 Métricas

- **Detecção** (conjunto B): dos sanduíches reais, quantos o stress test marcou como `sandwichable`. É a recall.
- **Precisão da perda:** a perda real da vítima comparada com a `worst_case_out` prevista. Erro em bps (mediana, p90).
- **Falso positivo** (conjunto A): swaps marcados como sanduicháveis em que o lucro do atacante é menor que o custo do bundle.
- **Latência:** p50 e p95 do stress test. Alvo: p95 < 50 ms por transação, com cache quente.
- **Camada de IA** (conjunto C): a explicação cita o valor exposto e a recomendação correta? Avaliar como no `AVALIACAO_E_TESTES.md`, separando o modelo puro dos overrides determinísticos.

## 7. Coordenação: frentes de trabalho

Cada frente tem **dono, entrada, saída e definição de pronto**. As frentes conversam só pelas interfaces abaixo, então dá para tocar em paralelo (pessoas ou agentes).

| ID | Frente | Dono sugerido | Entrada | Saída | Pronto quando |
|---|---|---|---|---|---|
| **MEV-1** | Modo "executar em snapshot" no aval-svm | Lucas | PR #1 | API interna `stress_run(snapshot, txs) → resultados` | testes com o `.so` real do Raydium/Orca carregado na VM |
| **MEV-2** | Detector de pool + clonagem de instrução (Raydium v4, Orca Whirlpool) | Lucas | MEV-1 | front-run e back-run sintéticos válidos | sanduíche sintético reproduz o lucro de um caso real conhecido (±5%) |
| **MEV-3** | Busca do pior caso + `/v1/stress` + CLI | Lucas | MEV-2 | endpoint conforme §4.4 | p95 < 50 ms; DEX não suportada → `supported:false` |
| **MEV-4** | Minerador de sanduíches reais (conjunto B) | **Luis** | blocos da mainnet via RPC | `results/sandwiches_real.jsonl` (bot, vítima, pool, slot, valores, estado anterior) | ≥ 50 sanduíches reais com perda da vítima calculada |
| **MEV-5** | Avaliação do stress test contra B, e distribuição em A | **Luis** | MEV-3 + MEV-4 | `results/mev_report.json` com as métricas da §6.3 | relatório reproduzível com seed e commit |
| **MEV-6** | Tipos novos no gerador do lab (conjunto C) e no benchmark | **Luis** | — | `sandwich_exposure` e `benign_tight_slippage` em `ATTACK_KINDS`/`BENIGN_KINDS`, pareados | casos entram no `09_SIMULAR_ATAQUES_SOLANA` com split holdout |
| **MEV-7** | Treino/ajuste da LoRA para explicar exposição | **Luis** | MEV-6 (+ rótulos de MEV-3) | adaptador novo + avaliação | explica o valor exposto e a recomendação em ≥ 90% do holdout; sem regressão nos ataques antigos |
| **MEV-8** | Finding `mev.sandwich_exposure` no VETO + regras do gate | Eric | MEV-3 | finding com severidade e recomendação | gate faz flag/deny conforme §5 em fixtures |
| **MEV-9** | Demo do pitch | Lucas | MEV-3, MEV-5 | slide: swap real → "Aval teria bloqueado: X SOL expostos" + atestação on-chain | roteiro de 60 s ensaiado |

**Ordem e paralelismo:**

- **Já:** MEV-1, MEV-4 e MEV-6 começam em paralelo, sem dependência entre si.
- **Depois:** MEV-2 → MEV-3, enquanto o Luis termina MEV-4 e MEV-6.
- **Na sequência:** MEV-5 e MEV-7, quando MEV-3 existir.
- **Por fim:** MEV-8 e MEV-9.

**Prazo realista para o hackathon** (submissão 12/out):

- MEV-1 a MEV-3 só com **Raydium AMM v4**.
- MEV-4 com ~50 casos reais.
- MEV-5 e MEV-9.
- MEV-6, MEV-7 e MEV-8 podem ficar como "próximo passo" no pitch, se não couberem.

**Para quem usa agentes de código:** cada linha da tabela vira uma tarefa independente, com o "Pronto quando" como critério de aceite. Os IDs `MEV-n` vão nas mensagens de commit, como já fazemos no board.

## 8. Regras de segurança (inegociáveis)

Este trabalho constrói **ataques simulados**. Para que nunca vire um bot de sanduíche:

1. **Só simulação.** Transações de atacante existem **apenas dentro da VM local**. O código de stress **não tem** caminho para assinar com chave real, enviar transação ou falar com o Block Engine da Jito.
2. **Carteira do atacante é sintética:** endereço fixo de teste, saldo criado com `set_account`. Não existe keypair real.
3. **Sem orderflow alheio.** Só analisamos a transação **do próprio agente**, antes da assinatura. Não escutamos transações de terceiros para atacá-las.
4. **Conjunto B é histórico:** blocos já finalizados, para medir. Nunca para agir.
5. Os testes do CI verificam a regra 1. Os módulos de stress **não importam** `chain::RpcChain::send` nem nada que envie.

## 9. Decisões abertas para o Luis

| # | Pergunta | Recomendação |
|---|---|---|
| D1 | v1 cobre quais DEXs? | **Raydium AMM v4** até 12/out; Orca Whirlpool logo depois |
| D2 | Limite de flag/deny (bps)? | flag a partir de **50 bps**, deny a partir de **200 bps** ou de **US$ 5** absolutos; calibrar com o conjunto A |
| D3 | Onde mora o minerador de sanduíches reais (MEV-4)? | em **Python, no lab** (`src/mev_miner.py`), usando o RPC configurado; vira `10_MINERAR_SANDUICHES.bat` |
| D4 | Treinar a LoRA agora (MEV-7) ou só depois do hackathon? | **depois**; para o pitch bastam o stress test determinístico e a avaliação contra casos reais |
| D5 | Gravar o resultado do stress test na atestação on-chain (PR #2)? | **sim**, um campo no registro do veredito; o batcher já ancora o que estiver na linha |
| D6 | RPC para mainnet (conjuntos A e B) | chave grátis da **Helius** (ou outro RPC com `getBlock`); o RPC público não aguenta o volume |

---

### Referências

- Fluxo da Jito (Block Engine, Relayer, atraso de 200 ms) e os exemplos de arbitragem, backrun e sanduíche: slides da **Helius**. Os prints estão guardados localmente, fora do repositório, por serem material de terceiros.
- Simulador: `svm/docs/2026-10-06-aval-svm-design.md` (PR #1). Atestação: `svm/docs/2026-10-06-aval-registry-design.md` (PR #2).

🤖 Proposta escrita com [Claude Code](https://claude.com/claude-code)
