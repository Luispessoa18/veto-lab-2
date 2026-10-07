#!/usr/bin/env bash
# Validador Solana local (custo zero) + deploy do aval_registry.
# Uso: registry/scripts/localnet.sh          sobe o validador e faz o deploy
#      registry/scripts/localnet.sh --stop   para o validador iniciado por este script
set -euo pipefail
cd "$(dirname "$0")/.."   # registry/

export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.avm/bin:$PATH"
RPC="http://127.0.0.1:8999"
DIR=".localnet"
PIDFILE="$DIR/validator.pid"
WALLET_KEY="${SOLANA_KEYPAIR:-$HOME/.config/solana/id.json}"
PROGRAM_KEY="target/deploy/aval_registry-keypair.json"
SO="../svm/tests/fixtures/aval_registry.so"

if [ "${1:-}" = "--stop" ]; then
  if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
    kill "$(cat "$PIDFILE")"
    echo "[OK] Validador local parado (pid $(cat "$PIDFILE"))."
  else
    echo "[OK] Nenhum validador iniciado por este script esta rodando."
  fi
  rm -f "$PIDFILE"
  exit 0
fi

command -v solana-test-validator >/dev/null || { echo "[ERRO] solana-test-validator nao encontrado. Instale o Solana CLI (Agave)."; exit 1; }
[ -f "$SO" ] || { echo "[ERRO] Faltando $SO"; exit 1; }
[ -f "$WALLET_KEY" ] || { echo "[ERRO] Carteira $WALLET_KEY nao encontrada (solana-keygen new)."; exit 1; }
if [ ! -f "$PROGRAM_KEY" ]; then
  echo "[ERRO] Faltando $PROGRAM_KEY (keypair do programa)."
  echo "       Sem ele o endereco do programa seria diferente do documentado (5t75hMEMtV5rEN7BuRu3pQfyu2yUc5DVMBFqvdLxoZN5)."
  echo "       Nao vou gerar um id novo em silencio; restaure o keypair ou aceite o novo endereco conscientemente."
  exit 1
fi

if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
  echo "[ERRO] Validador ja esta rodando (pid $(cat "$PIDFILE")). Use --stop primeiro."; exit 1
fi

mkdir -p "$DIR"
echo "[1/3] Iniciando validador local em $RPC (ledger novo)..."
solana-test-validator --reset --quiet --ledger "$DIR" --rpc-port 8999 --faucet-port 9909 \
  >"$DIR/validator.log" 2>&1 &
VPID=$!

for i in $(seq 1 60); do
  if solana cluster-version --url "$RPC" >/dev/null 2>&1; then break; fi
  if ! kill -0 "$VPID" 2>/dev/null; then echo "[ERRO] Validador caiu. Veja $DIR/validator.log"; exit 1; fi
  if [ "$i" = 60 ]; then echo "[ERRO] Validador nao respondeu em 60 s. Veja $DIR/validator.log"; kill "$VPID" 2>/dev/null || true; exit 1; fi
  sleep 1
done
# o --reset apaga o conteudo do ledger, entao o pid so e gravado depois que o RPC responde
echo "$VPID" >"$PIDFILE"
echo "      pronto: solana $(solana cluster-version --url "$RPC")"

WALLET="$(solana-keygen pubkey "$WALLET_KEY")"
echo "[2/3] Airdrop de 2.5 SOL simulados (so existem neste validador) para $WALLET..."
solana airdrop 2.5 "$WALLET" --url "$RPC"
solana balance "$WALLET" --url "$RPC"

echo "[3/3] Deploy do aval_registry (upgrade authority = $WALLET)..."
solana program deploy --url "$RPC" --keypair "$WALLET_KEY" --program-id "$PROGRAM_KEY" "$SO"
solana program show "$(solana-keygen pubkey "$PROGRAM_KEY")" --url "$RPC"

echo "[OK] Localnet pronta. Use: export AVAL_REGISTRY_RPC=$RPC"
echo "     Para parar: registry/scripts/localnet.sh --stop"
