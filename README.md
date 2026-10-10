# VETO LAB — benchmark de segurança para carteiras

API local que verifica transações (EVM/Solana) e prompts de agentes em camadas: blacklist → Prompt Guard → IA (VETO LFM2.5 350M) → simulação. Detalhes da metodologia e limitações em [LEIA-ME.md](LEIA-ME.md).

Para interpretar corretamente as métricas e planejar avaliações independentes,
consulte [AVALIACAO_E_TESTES.md](AVALIACAO_E_TESTES.md).

## O que está (e o que não está) no repositório

| Item | No git? | Como obter |
|---|---|---|
| Código (`src/`, `tests/`, `config/`, `.bat`, `.sh`) | ✅ | — |
| VETO LFM2.5 350M com LoRA embutido | ❌ | coloque em `models/veto_lfm2_5_350m_aave_f16.gguf` |
| Gemma 3 4B + LoRA externo | opcional/fallback | — |
| Gemma 3 4B IT `Q4_K_M` (~2.5 GB) | ❌ | `BAIXAR_MODELOS.bat` / `./baixar_modelos.sh` |
| Prompt Guard 2 86M (Meta, ~1.1 GB, acesso restrito) | ❌ | mesmo script, com `HF_TOKEN` |
| llama.cpp (`llama/`) | ❌ | [releases do llama.cpp](https://github.com/ggml-org/llama.cpp/releases) |
| `.venv`, `tools/`, `external/` | ❌ | criados pelos scripts |

O Prompt Guard é um modelo restrito: aceite a licença em <https://huggingface.co/meta-llama/Llama-Prompt-Guard-2-86M>, gere um token em <https://huggingface.co/settings/tokens> e defina `HF_TOKEN` antes de rodar o script de download.

## Windows

1. Instale Python 3.11+, Git e (opcional) [Foundry](https://getfoundry.sh) para o Anvil.
2. Baixe o llama.cpp para Windows (CPU ou CUDA) e extraia **todo o pacote** em `llama\` (precisa de `llama\llama-server.exe` e das DLLs).
3. Baixe os modelos:
   ```bat
   set HF_TOKEN=hf_xxx
   BAIXAR_MODELOS.bat
   ```
4. Crie o ambiente Python único (usado pelo benchmark, API e Prompt Guard):
   ```bat
   py -3 -m venv .venv
   .venv\Scripts\python.exe -m pip install -r requirements-guard.txt
   ```
   No Windows, `01_INSTALAR.bat` executa esses passos automaticamente.
5. Suba tudo: `00_INICIAR_TUDO.bat` → abre o Swagger em <http://127.0.0.1:8070/docs> e o painel em <http://127.0.0.1:8070/admin>. O painel acompanha requisições, latências e o relatório detalhado do benchmark Solana. O Anvil é opcional e só é iniciado quando está instalado; ele não é necessário para o fluxo Solana.

## Console do Aval (`console/`)

Painel do operador: Live gate, Overview, Issues, Lab traffic (o tráfego desta API, lido de `/admin/requests`), Runs (execuções em lote inteiras de `results/runs/`, via `/admin/runs`), Actions com gráficos por período, e System com a saúde da API (8070), do aval-svm (8899) e do motor Veto (5173).

```bash
cd console && npm install    # uma vez (Node 20+)
./iniciar_tudo.sh            # sobe tudo, inclusive o console em http://127.0.0.1:5190
```

Detalhes em [console/README.md](console/README.md).

Os demais `.bat` (`02_SERVIDOR`, `03_GERAR_CASOS`, `04_RODAR_BENCHMARK`, ...) seguem descritos no [LEIA-ME.md](LEIA-ME.md).

## Linux

### 1. Dependências do sistema
```bash
# Debian/Ubuntu
sudo apt update && sudo apt install -y git curl python3 python3-venv build-essential cmake
```

### 2. Clonar o projeto
```bash
git clone https://github.com/Luispessoa18/veto-lab-2.git
cd veto-lab-2
chmod +x *.sh
```

### 3. llama.cpp (`llama-server`)
Compile a partir do código-fonte (fora do projeto):
```bash
git clone https://github.com/ggml-org/llama.cpp ~/llama.cpp
cd ~/llama.cpp
cmake -B build                       # CPU
# cmake -B build -DGGML_CUDA=ON      # GPU NVIDIA (requer CUDA Toolkit)
cmake --build build --config Release -j
cd -
export LLAMA_SERVER=~/llama.cpp/build/bin/llama-server
```
Alternativas: baixar o binário Linux em [releases](https://github.com/ggml-org/llama.cpp/releases) ou `brew install llama.cpp`.

### 4. Modelos
```bash
HF_TOKEN=hf_xxx ./baixar_modelos.sh
```

### 5. Ambiente Python
```bash
python3 -m venv .venv
.venv/bin/pip install -r requirements-guard.txt
# so CPU (download bem menor do torch):
# .venv/bin/pip install torch --index-url https://download.pytorch.org/whl/cpu
```

### 6. Anvil (Foundry)
```bash
curl -L https://foundry.paradigm.xyz | bash
~/.foundry/bin/foundryup
```

### 7. Subir tudo
```bash
./iniciar_tudo.sh                 # CPU por padrão
VETO_NGL=99 ./iniciar_tudo.sh     # offload para GPU
```
Sobe llama.cpp (18080), Anvil opcional (8545) e API + Prompt Guard (8070). Logs em `results/*.log`. Swagger: <http://127.0.0.1:8070/docs>. `CTRL+C` encerra tudo.

O LFM já contém o ajuste VETO e não recebe `--lora`. Para voltar temporariamente
ao Gemma com adaptador externo no Windows:

```bat
set "VETO_MODEL=models\gemma-3-4b-it-Q4_K_M.gguf"
set "VETO_LORA=models\VETO-Security-LoRA-F16.gguf"
set "VETO_NGL=99"
00_INICIAR_TUDO.bat
```

Para subir cada serviço manualmente:
```bash
$LLAMA_SERVER -m models/veto_lfm2_5_350m_aave_f16.gguf \
  --host 127.0.0.1 --port 18080 -c 4096 -np 2 -ngl 0 --jinja
anvil --host 127.0.0.1 --port 8545
.venv/bin/python -m src.unified_api --host 127.0.0.1 --port 8070
# Prompt Guard standalone (usado pelo benchmark):
.venv/bin/python -m src.guard_server --host 127.0.0.1 --port 8090 --threshold 0.90
```

### 8. Benchmarks e utilitários (equivalentes dos `.bat`)
| Windows | Linux |
|---|---|
| `03_GERAR_CASOS.bat` | `.venv/bin/python -m src.main generate --count 1200` |
| `04_RODAR_BENCHMARK.bat` | `.venv/bin/python -m src.main run` |
| `05_RELATORIO.bat` | `.venv/bin/python -m src.main report` |
| `06_CATALOGO_HISTORICO.bat` | `.venv/bin/python -m src.main catalog --fetch-rpc` |
| `07_PROMPT_GUARD.bat` | `.venv/bin/python -m src.guard_server --port 8090 --threshold 0.90` |
| `08_API_UNICA.bat` | `.venv/bin/python -m src.unified_api --port 8070` |
| `09_SIMULAR_ATAQUES_SOLANA.bat` | `.venv/bin/python -m src.solana_attack_simulator --api http://127.0.0.1:8070` |
| testes | `.venv/bin/python -m unittest discover -s tests` |

> **Guia passo a passo para configurar e testar:** [CONFIGURAR_AVAL.md](CONFIGURAR_AVAL.md)

## aval-svm — simulação Solana local (Rust)

A camada `solana_simulation` roda as transações numa VM Solana local (LiteSVM) em vez de
chamar o RPC público a cada verificação. As contas vêm do RPC uma vez, ficam em cache
(2 s; programas são relidos a cada 60 s, `program_ttl_ms`, para pegar upgrades) e a simulação
roda em milissegundos. Se o aval-svm não estiver rodando, não responder, ou não suportar um
programa, a API volta sozinha para o RPC público.

Requisitos: Rust (rustc >= 1.97.1; se já tiver Rust, rode `rustup update`). No Linux, instale
`build-essential`; no Windows, use o `rustup-init.exe` (https://rustup.rs) e o Visual Studio
Build Tools com o componente "Desenvolvimento para desktop com C++" (MSVC).

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y   # uma vez
cd svm && cargo build --release && cd ..
./svm/target/release/aval-svm serve --config svm/aval-svm.toml          # porta 8899
```

- A porta 8899 também é a padrão do `solana-test-validator`. Para usar outra, defina
  `AVAL_LISTEN=127.0.0.1:8898` e mude `aval_svm_url` em `config/settings.json` para a mesma
  porta. Se outro servidor responder na porta (sem o bloco `aval`), a API usa o RPC público.

- Compatível com JSON-RPC: qualquer cliente que fala `simulateTransaction` (inclusive o VETO
  em TypeScript) só precisa apontar a URL para `http://127.0.0.1:8899`.
- `POST /v1/project` devolve a **projeção**: saldos SOL/tokens antes e depois, mudanças de
  dono/delegate e contas criadas/fechadas.
- Toda resposta traz `aval.digest` (sha256 da mensagem, igual ao `messageDigest` do VETO) e
  o slot do estado usado (`stateSlot`, o mais novo; `stateSlotMin`, o mais antigo entre as contas
  que não são programas — iguais significa que o estado veio todo de um só slot).
- `getMultipleAccounts` com `encoding: "base64"` (sem `dataSlice`/`minContextSlot`, com ao menos uma
  conta) sai do mesmo cache da simulação — em geral no mesmo slot dela, mas sem garantia; outras
  formas vão para o upstream.
- Upstream: `AVAL_UPSTREAM_URL` (padrão devnet). Para mainnet use um RPC próprio (Helius etc.).
- Medir contra o RPC: `aval-svm shadow --upstream <url> --count 200`.
- Usa o `Clock` e o `EpochSchedule` do cluster, e verifica os precompiles ed25519/secp256k1.
  Transações maiores que 4096 bytes são recusadas (`-32602`).
- Limitações conhecidas: `minContextSlot` é ignorado; `sigVerify: true` é recusado (simule a
  transação sem assinatura); o `replacementBlockhash` é gerado pela VM local, não é um blockhash
  do cluster — não assine transações com ele.

Desenho: `svm/docs/2026-10-06-aval-svm-design.md`.

## aval-registry — atestação on-chain dos vereditos

O `aval-svm anchor` lê o arquivo de registros do VETO (JSONL, um veredito por linha), agrupa as
linhas novas em lotes, calcula a raiz Merkle de cada lote e grava a raiz no programa Anchor
`aval_registry` (`registry/`). O `aval-svm verify` prova que uma linha específica está sob uma
raiz gravada na cadeia. Desenho: `svm/docs/2026-10-06-aval-registry-design.md`.

**O que `VERIFIED` prova:** os bytes exatos da linha N do arquivo foram comprometidos como linha N
pelo registry da authority X, no lote k, no slot S ou antes dele. Os lotes são contíguos, então
uma linha ancorada não pode mudar e não existe buraco no meio.

**O que `VERIFIED` NÃO prova:**

- que o veredito está correto (só que ele não foi alterado depois);
- que o horário gravado no próprio registro é verdadeiro (a prova só garante "até o slot S");
- que todo veredito foi escrito no arquivo: o operador pode omitir vereditos ou deixar o final
  do arquivo sem ancorar;
- linhas ainda não ancoradas: podem ser alteradas dentro do `--interval-secs` (padrão 30 s) ou
  enquanto o batcher estiver parado.

Cuidados:

- Sempre passe `--authority`. Sem ele o verificador confia no registry citado no arquivo de provas.
- O verificador confia no RPC que consulta (leitura em commitment `finalized`).
- As provas só existem em `<registros>.proofs.jsonl`: faça backup. O arquivo de registros deve
  ser somente de acréscimo (append-only).
- O programa é atualizável pela carteira local até ser finalizado
  (`solana program set-upgrade-authority <programa> --final`, irreversível; **não foi feito**).
- Custo: cerca de 0,0017 SOL de aluguel por lote em devnet (grátis na localnet).
- Códigos de saída: `0` verificado, `1` NÃO verificado, `2` não deu para checar (erro de RPC, arquivo ou argumentos, ou `PENDING`: tente de novo).

### Rodar na localnet (custo zero)

Requer o Solana CLI (Agave). O script sobe um `solana-test-validator` na porta 8999, faz um airdrop
de 2,5 SOL simulados e deixa o programa `5t75hMEMtV5rEN7BuRu3pQfyu2yUc5DVMBFqvdLxoZN5` (o `.so` e o id
estão em `svm/tests/fixtures/`) carregado. Com o keypair do programa em `registry/target/deploy/`
(nunca versionado) ele faz um `solana program deploy` de verdade; num clone limpo, sem esse keypair,
carrega o programa no genesis com o mesmo id e a sua carteira como upgrade authority. Se
`~/.config/solana/id.json` não existir, o script cria uma carteira nova (sem exibir a chave).
`--demo-records` escreve `results/demo-records.jsonl` (10 linhas sintéticas) para os comandos abaixo:

```bash
registry/scripts/localnet.sh            # sobe o validador e publica o programa
registry/scripts/localnet.sh --demo-records   # results/demo-records.jsonl
export AVAL_REGISTRY_RPC=http://127.0.0.1:8999
./svm/target/release/aval-svm anchor --records results/demo-records.jsonl \
    --keypair ~/.config/solana/id.json --once
./svm/target/release/aval-svm verify --records results/demo-records.jsonl --line 7 \
    --authority $(solana-keygen pubkey ~/.config/solana/id.json)
registry/scripts/localnet.sh --stop     # para o validador
```

A leitura do `verify` é `finalized`, que no validador local demora uns 15 s depois do `anchor`.
Nesse intervalo ele imprime `PENDING: batch k is confirmed but not finalized yet — retry in ~15 s`
e sai com código `2`: não é adulteração, só repetir o comando. `NOT VERIFIED: ... not found on chain`
(código `1`) fica para o lote que não existe nem em `confirmed`.

Saída real de uma execução (10 linhas sintéticas, depois mais 3):

```text
anchored batch 0: lines 0–9 (10 records) tx 4u6pFwmngi9DG35Yq53rnrboJQNURmDeLfH6e3FgmKHFM7ZheKTm1rubuTq9p3bEBj7TAB5MncCP87vLhX2u77Di
VERIFIED line 7 — batch 0, slot 26, 2026-10-07T02:38:23Z, tx 4u6pFw…2u77Di, registry 5DUzCe7nW23RRWHzqvNHPs6V7biNP4eqEPbrj5YFxnGh (authority 3Q2guAHRjtUdhbTvvXzztpjEdFG8RpMm15fv6Lho4mzD)   # saída 0
NOT VERIFIED: leaf mismatch: line 7 hashes to c09da1d0…, the proof entry has c3c7e96c…           # linha 7 adulterada numa cópia, saída 1
NOT VERIFIED: proof entry names registry 5DUzCe7n…, but the registry of authority 5t75hMEM…(id do programa, errada) is EqktkaAX…   # saída 1
anchored batch 1: lines 10–12 (3 records) tx 22srm8iGWSzzK6H7VEn34fwRVzRgvDrEoM4yXqcQnyPQmJEApxMrhwjoSs4FpHuyWiTEY4h4vRQrbWsUyBKDYwce
VERIFIED line 11 — batch 1, slot 32, 2026-10-07T02:38:26Z, tx 22srm8…BKDYwce, registry 5DUzCe7nW23RRWHzqvNHPs6V7biNP4eqEPbrj5YFxnGh (authority 3Q2guAHRjtUdhbTvvXzztpjEdFG8RpMm15fv6Lho4mzD)   # saída 0
```

Os mesmos comandos funcionam na devnet com `AVAL_REGISTRY_RPC` sem definir (o padrão é a devnet);
nesse caso é preciso ter SOL de devnet e fazer o deploy do programa lá.

## Avaliação adversarial

`03_GERAR_CASOS.bat` cria 600 casos-base e aproximadamente 20% de equivalentes
metamórficos. Cada execução usa uma seed aleatória registrada em
`results/solana_cases.jsonl`, distribui casos entre `development` e `holdout` e
embaralha tipos, valores, limites e textos. `09_SIMULAR_ATAQUES_SOLANA.bat` executa
exatamente o arquivo preparado, evitando gerar outro conjunto durante a medição.

O relatório separa acurácia do pipeline, decisão bruta do LFM, overrides,
componentes, baselines triviais, development/holdout e estabilidade metamórfica.
Consulte [AVALIACAO_E_TESTES.md](AVALIACAO_E_TESTES.md) para ablações e limites da
metodologia.

## RPC Solana

`09_SIMULAR_ATAQUES_SOLANA.bat` executa casos sintéticos e sobe um RPC mock somente
em `127.0.0.1`; ele não transmite transações nem precisa de acesso à mainnet.

Para verificar uma transação Solana real já serializada, configure um endpoint RPC
antes de iniciar a API:

```bat
set "SOLANA_RPC_URL=https://seu-endpoint-solana"
08_API_UNICA.bat
```

Também é possível alterar `solana_rpc_url` em `config/settings.json`. A API chama
apenas `simulateTransaction`, com `sigVerify=false` e
`replaceRecentBlockhash=true`; ela não chama `sendTransaction`. Não coloque chaves
privadas no arquivo de configuração.

Por padrão, `config/settings.json` já contém os RPCs públicos oficiais de Devnet,
Testnet e Mainnet. Devnet é o cluster padrão. Para selecionar outro endpoint sem
editar o arquivo:

```bat
set "SOLANA_CLUSTER=testnet"
08_API_UNICA.bat
```

Ou informe `"cluster": "mainnet"`, `"testnet"` ou `"devnet"` dentro do objeto
`solana` da requisição. A transação serializada deve ter sido construída para o
cluster escolhido. Os endpoints públicos têm limite de uso e não oferecem SLA.

`03_GERAR_CASOS`/`catalog` clonam o DeFiHackLabs em `external/` (precisa de `git`).

## Endpoints
- `POST /v1/transactions/verify` — blacklist → camada 1 (política + IA) → simulação Anvil.
- `POST /v1/agent/run` — Prompt Guard antes da IA; injection retorna `decision=BLOCK`.
- `GET /health` — saúde e rotas.

Auditoria em `results/api_audit.jsonl`. Blacklist em `config/blacklist.json`.

> Uso apenas local (127.0.0.1). Nunca use chaves privadas reais nem rode contra a mainnet.
