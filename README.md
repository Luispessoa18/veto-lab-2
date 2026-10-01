# VETO LAB — benchmark de carteira com Gemma 3 + LoRA VETO Security

API local que verifica transações (EVM/Solana) e prompts de agentes em camadas: blacklist → Prompt Guard → IA (Gemma 3 4B + LoRA VETO) → simulação no Anvil. Detalhes da metodologia e limitações em [LEIA-ME.md](LEIA-ME.md).

## O que está (e o que não está) no repositório

| Item | No git? | Como obter |
|---|---|---|
| Código (`src/`, `tests/`, `config/`, `.bat`, `.sh`) | ✅ | — |
| LoRA `models/VETO-Security-LoRA-F16.gguf` (~60 MB) | ✅ | — |
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
4. Crie o ambiente Python (usado pela API e pelo Prompt Guard):
   ```bat
   py -3 -m venv .venv-lora
   .venv-lora\Scripts\python.exe -m pip install -r requirements-guard.txt
   ```
   E o ambiente leve do benchmark: `01_INSTALAR.bat`.
5. Suba tudo: `00_INICIAR_TUDO.bat` → abre o Swagger em <http://127.0.0.1:8070/docs>.

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
./iniciar_tudo.sh          # GPU
NGL=0 ./iniciar_tudo.sh    # somente CPU
```
Sobe llama.cpp (8080), Anvil (8545) e API + Prompt Guard (8070). Logs em `results/*.log`. Swagger: <http://127.0.0.1:8070/docs>. `CTRL+C` encerra tudo.

Para subir cada serviço manualmente:
```bash
$LLAMA_SERVER -m models/gemma-3-4b-it-Q4_K_M.gguf --lora models/VETO-Security-LoRA-F16.gguf \
  --host 127.0.0.1 --port 8080 -c 8192 -np 2 -ngl 99 --jinja
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

`03_GERAR_CASOS`/`catalog` clonam o DeFiHackLabs em `external/` (precisa de `git`).

## Endpoints
- `POST /v1/transactions/verify` — blacklist → camada 1 (política + IA) → simulação Anvil.
- `POST /v1/agent/run` — Prompt Guard antes da IA; injection retorna `decision=BLOCK`.
- `GET /health` — saúde e rotas.

Auditoria em `results/api_audit.jsonl`. Blacklist em `config/blacklist.json`.

> Uso apenas local (127.0.0.1). Nunca use chaves privadas reais nem rode contra a mainnet.
