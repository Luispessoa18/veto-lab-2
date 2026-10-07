#!/usr/bin/env bash
# Validador Solana local (custo zero) + aval_registry carregado.
# Uso: registry/scripts/localnet.sh                 sobe o validador e publica o programa
#      registry/scripts/localnet.sh --stop          para o validador iniciado por este script
#      registry/scripts/localnet.sh --demo-records  escreve results/demo-records.jsonl (10 linhas sinteticas)
set -euo pipefail
cd "$(dirname "$0")/.."   # registry/

export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.avm/bin:$PATH"
RPC="http://127.0.0.1:8999"
DIR=".localnet"
PIDFILE="$DIR/validator.pid"
WALLET_KEY="${SOLANA_KEYPAIR:-$HOME/.config/solana/id.json}"
PROGRAM_KEY="target/deploy/aval_registry-keypair.json"
SO="../svm/tests/fixtures/aval_registry.so"
ID_FILE="../svm/tests/fixtures/aval_registry.id"

if [ "${1:-}" = "--demo-records" ]; then
  mkdir -p ../results
  python3 - <<'PY' > ../results/demo-records.jsonl
import json, hashlib
for i in range(10):
    print(json.dumps({"at": 1791300000+i, "decision": ["allow","flag","deny"][i%3], "messageDigest": hashlib.sha256(str(i).encode()).hexdigest(), "policyVersion": "demo-1", "findings": []}))
PY
  echo "[OK] results/demo-records.jsonl escrito (10 linhas sinteticas)."
  exit 0
fi

if [ "${1:-}" = "--stop" ]; then
  if [ -f "$PIDFILE" ]; then
    pid="$(cat "$PIDFILE")"
    if [ "$(ps -p "$pid" -o comm= 2>/dev/null | xargs basename 2>/dev/null || true)" = "solana-test-validator" ]; then
      kill "$pid"
      for _ in $(seq 1 10); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
      if kill -0 "$pid" 2>/dev/null; then echo "[ERRO] Validador (pid $pid) nao saiu em 10 s."; exit 1; fi
      echo "[OK] Validador local parado (pid $pid)."
    else
      echo "[OK] O pid $pid nao e um solana-test-validator; nada foi encerrado."
    fi
    rm -f "$PIDFILE"
  else
    echo "[OK] Nenhum validador iniciado por este script esta rodando."
  fi
  exit 0
fi

command -v solana-test-validator >/dev/null || { echo "[ERRO] solana-test-validator nao encontrado. Instale o Solana CLI (Agave)."; exit 1; }
[ -f "$SO" ] || { echo "[ERRO] Faltando $SO"; exit 1; }
[ -f "$ID_FILE" ] || { echo "[ERRO] Faltando $ID_FILE"; exit 1; }
PROGRAM_ID="$(tr -d '[:space:]' < "$ID_FILE")"

if [ ! -f "$WALLET_KEY" ]; then
  echo "[..] Carteira $WALLET_KEY nao existe; criando uma nova (a chave secreta nao e exibida)."
  mkdir -p "$(dirname "$WALLET_KEY")"
  solana-keygen new --no-bip39-passphrase --silent --outfile "$WALLET_KEY" >/dev/null
fi
WALLET="$(solana-keygen pubkey "$WALLET_KEY")"

if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
  echo "[ERRO] Validador ja esta rodando (pid $(cat "$PIDFILE")). Use --stop primeiro."; exit 1
fi

# Com o keypair do programa: deploy real (solana program deploy).
# Sem ele (clone limpo): o programa entra no genesis, com o mesmo id (embutido no .so) e upgrade authority = carteira.
GENESIS=()
if [ -f "$PROGRAM_KEY" ]; then
  MODE=deploy
  [ "$(solana-keygen pubkey "$PROGRAM_KEY")" = "$PROGRAM_ID" ] || { echo "[ERRO] $PROGRAM_KEY nao corresponde ao id $PROGRAM_ID."; exit 1; }
else
  MODE=genesis
  echo "[..] Sem $PROGRAM_KEY: o programa $PROGRAM_ID sera carregado no genesis (mesmo id, authority = $WALLET)."
  GENESIS=(--upgradeable-program "$PROGRAM_ID" "$SO" "$WALLET")
fi

mkdir -p "$DIR"
echo "[1/3] Iniciando validador local em $RPC (ledger novo)..."
solana-test-validator --reset --quiet --ledger "$DIR" --rpc-port 8999 --faucet-port 9909 ${GENESIS[@]+"${GENESIS[@]}"} \
  >"$DIR/validator.log" 2>&1 &
VPID=$!
# Se algo falhar antes do pid ser gravado, nao deixa o validador orfao.
cleanup() { kill "$VPID" 2>/dev/null || true; }
trap cleanup INT TERM EXIT

for i in $(seq 1 60); do
  if solana cluster-version --url "$RPC" >/dev/null 2>&1; then break; fi
  if ! kill -0 "$VPID" 2>/dev/null; then echo "[ERRO] Validador caiu. Veja $DIR/validator.log"; exit 1; fi
  if [ "$i" = 60 ]; then echo "[ERRO] Validador nao respondeu em 60 s. Veja $DIR/validator.log"; exit 1; fi
  sleep 1
done
# o --reset apaga o conteudo do ledger, entao o pid so e gravado depois que o RPC responde
echo "$VPID" >"$PIDFILE"
trap - INT TERM EXIT
echo "      pronto: solana $(solana cluster-version --url "$RPC")"

echo "[2/3] Airdrop de 2.5 SOL simulados (so existem neste validador) para $WALLET..."
solana airdrop 2.5 "$WALLET" --url "$RPC"
solana balance "$WALLET" --url "$RPC"

if [ "$MODE" = deploy ]; then
  echo "[3/3] Deploy do aval_registry (upgrade authority = $WALLET)..."
  solana program deploy --url "$RPC" --keypair "$WALLET_KEY" --program-id "$PROGRAM_KEY" "$SO"
else
  echo "[3/3] aval_registry ja carregado no genesis."
fi
solana program show "$PROGRAM_ID" --url "$RPC"

echo "[OK] Localnet pronta. Use: export AVAL_REGISTRY_RPC=$RPC"
echo "     Para parar: registry/scripts/localnet.sh --stop"
