#!/usr/bin/env bash
# Baixa os modelos grandes que nao ficam no git (equivalente ao BAIXAR_MODELOS.bat).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p models

GEMMA=models/gemma-3-4b-it-Q4_K_M.gguf
GEMMA_URL="https://huggingface.co/ggml-org/gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-Q4_K_M.gguf?download=true"
GUARD=models/Llama-Prompt-Guard-2-86M
GUARD_REPO=meta-llama/Llama-Prompt-Guard-2-86M

if [ -f "$GEMMA" ]; then
  echo "[OK] Gemma ja existe: $GEMMA"
else
  echo "[1/2] Baixando Gemma 3 4B IT Q4_K_M (~2.5 GB)..."
  curl -fL --retry 5 -C - -o "$GEMMA" "$GEMMA_URL"
fi

if [ -f "$GUARD/model.safetensors" ]; then
  echo "[OK] Prompt Guard ja existe: $GUARD"
elif [ -z "${HF_TOKEN:-}" ]; then
  echo "[AVISO] HF_TOKEN nao definido; pulando Prompt Guard (repositorio restrito da Meta)."
  echo "Aceite a licenca em https://huggingface.co/$GUARD_REPO e rode: HF_TOKEN=hf_xxx ./baixar_modelos.sh"
else
  echo "[2/2] Baixando Prompt Guard 2 86M..."
  mkdir -p "$GUARD"
  for f in config.json model.safetensors special_tokens_map.json tokenizer.json tokenizer_config.json; do
    echo "  - $f"
    curl -fL --retry 5 -C - -H "Authorization: Bearer $HF_TOKEN" -o "$GUARD/$f" \
      "https://huggingface.co/$GUARD_REPO/resolve/main/$f?download=true"
  done
fi

echo "Pronto. O LoRA ja vem no repositorio: models/VETO-Security-LoRA-F16.gguf"
