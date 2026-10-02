#!/usr/bin/env bash
# Equivalente Linux do 00_INICIAR_TUDO.bat: llama.cpp + Anvil + API/Prompt Guard.
set -euo pipefail
cd "$(dirname "$0")"

LLAMA="${LLAMA_SERVER:-$(command -v llama-server || echo llama/llama-server)}"
MODEL=models/gemma-3-4b-it-Q4_K_M.gguf
LORA=models/VETO-Security-LoRA-F16.gguf
PY=.venv/bin/python
NGL="${NGL:-99}"   # use NGL=0 para rodar so na CPU
mkdir -p results

[ -x "$LLAMA" ] || { echo "[ERRO] llama-server nao encontrado (defina LLAMA_SERVER=/caminho/llama-server)"; exit 1; }
[ -f "$MODEL" ] || { echo "[ERRO] Faltando $MODEL. Rode ./baixar_modelos.sh"; exit 1; }
[ -f "$LORA" ]  || { echo "[ERRO] Faltando $LORA"; exit 1; }
[ -x "$PY" ]    || { echo "[ERRO] Faltando .venv. Veja o README (instalacao Linux)"; exit 1; }
[ -f models/Llama-Prompt-Guard-2-86M/model.safetensors ] || { echo "[ERRO] Prompt Guard ausente. Rode HF_TOKEN=... ./baixar_modelos.sh"; exit 1; }
ANVIL="$(command -v anvil || echo "$HOME/.foundry/bin/anvil")"
[ -x "$ANVIL" ] || { echo "[ERRO] anvil nao encontrado. Instale o Foundry: curl -L https://foundry.paradigm.xyz | bash && foundryup"; exit 1; }

pids=()
trap 'echo; echo Encerrando...; kill "${pids[@]}" 2>/dev/null || true' EXIT INT TERM

echo "Iniciando llama.cpp na porta 18080 (log: results/llama.log)..."
"$LLAMA" -m "$MODEL" --lora "$LORA" --host 127.0.0.1 --port 18080 -c 8192 -np 2 -ngl "$NGL" --jinja > results/llama.log 2>&1 &
pids+=($!)

echo "Iniciando Anvil na porta 8545 (log: results/anvil.log)..."
"$ANVIL" --host 127.0.0.1 --port 8545 > results/anvil.log 2>&1 &
pids+=($!)

echo "Iniciando API + Prompt Guard na porta 8070 (log: results/api.log)..."
"$PY" -m src.unified_api --host 127.0.0.1 --port 8070 > results/api.log 2>&1 &
pids+=($!)

for _ in $(seq 1 60); do
  if curl -fs http://127.0.0.1:8070/health >/dev/null; then
    echo; echo "Tudo pronto. Swagger: http://127.0.0.1:8070/docs  (CTRL+C para parar)"
    wait
    exit 0
  fi
  sleep 2
done
echo "[ERRO] A API nao respondeu em 120 segundos. Veja results/api.log"
exit 1
