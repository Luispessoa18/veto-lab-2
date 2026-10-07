# Configurar o Aval na sua máquina

Guia passo a passo para rodar e testar as duas peças em Rust do Aval:

- **aval-svm**: simulação Solana local (LiteSVM). O lab usa no lugar do RPC público, em milissegundos.
- **aval-registry**: atestação on-chain dos vereditos. Prova que um veredito foi registrado e não foi editado depois.

Tudo aqui é **de graça**. Nada exige SOL de verdade.

---

## 0. Antes de começar

1. **Ordem de merge:** primeiro o **PR #1** (aval-svm), depois o **PR #2** (aval-registry). O #2 é construído em cima do #1.
2. **CI:** em cada PR, clique em **"Approve and run workflows"**. O GitHub só roda os testes de quem contribui por fork depois dessa aprovação. O CI roda os 142 testes em Rust (incluindo o programa on-chain de verdade dentro do LiteSVM), o clippy e a suíte do lab. **Verde no CI já é o teste mais forte**, e você não precisa instalar nada para ver.
3. **Windows:** use **WSL2 (Ubuntu)** para tudo que for Solana (validador local, CLI, build do programa). O `solana-test-validator` não funciona bem no Windows nativo. O lab em Python continua funcionando pelos `.bat`.

---

## 1. Instalar (uma vez)

No Linux ou no WSL2:

```bash
sudo apt update && sudo apt install -y build-essential pkg-config libssl-dev git curl python3 python3-venv

# Rust (precisa de rustc >= 1.97.1)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup update && rustc --version

# Solana CLI (validador local + carteira)
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
solana --version
```

Coloque o `export PATH=...` no seu `~/.bashrc` para não precisar repetir.

O **Anchor CLI não é necessário.** O programa já vem compilado em `svm/tests/fixtures/aval_registry.so`, com o hash conferido por teste. Só instale o Anchor se for recompilar o programa (seção 7).

---

## 2. Compilar e testar

```bash
git clone https://github.com/Luispessoa18/veto-lab-2.git && cd veto-lab-2
cd svm
cargo build --release          # a 1ª vez demora (baixa e compila o runtime Solana)
cargo test                     # esperado: todos passam (~142)
cargo clippy --all-targets -- -D warnings
cd ..
```

Se algo falhar aqui, pare e mande a saída: o resto depende disso.

---

## 3. aval-svm no lab (simulação rápida)

```bash
./svm/target/release/aval-svm serve        # porta 8899, upstream = devnet
```

Em outro terminal, suba o lab como sempre (`./iniciar_tudo.sh`, ou `00_INICIAR_TUDO.bat` no Windows). Os dois scripts já iniciam o aval-svm sozinhos quando o binário existe.

**Como saber que está funcionando:**

- No trace da camada `solana_simulation`, aparece `engine: "aval-svm"` e um campo `aval` com `digest`, `stateSlot` e `elapsedUs`.
- **Teste de queda:** pare o aval-svm e rode de novo. Deve aparecer `engine: "rpc"`, e o lab continua funcionando (volta sozinho para o RPC público).

Testes úteis:

```bash
curl -s localhost:8899/health                                   # {"ok":true}
./svm/target/release/aval-svm digest <tx em base64>             # mesmo hash que o messageDigest do VETO
```

**Opcional, medir fidelidade:** compara o aval-svm com o RPC real em transações de um bloco. Para 200 transações na mainnet, use um RPC próprio (ex.: chave grátis da Helius), senão o RPC público devolve 429.

```bash
AVAL_UPSTREAM_URL="https://mainnet.helius-rpc.com/?api-key=SUA_CHAVE" ./svm/target/release/aval-svm shadow --count 200
```

---

## 4. aval-registry na localnet (custo zero)

Validador Solana local na porta **8999** (não conflita com o aval-svm na 8899), com 2,5 SOL simulados e o programa já publicado:

```bash
registry/scripts/localnet.sh                  # sobe o validador, cria a carteira se faltar, publica o programa
registry/scripts/localnet.sh --demo-records   # gera results/demo-records.jsonl (10 vereditos de exemplo)

export AVAL_REGISTRY_RPC=http://127.0.0.1:8999
AUTH=$(solana-keygen pubkey ~/.config/solana/id.json)

./svm/target/release/aval-svm anchor --records results/demo-records.jsonl --keypair ~/.config/solana/id.json --once
sleep 20
./svm/target/release/aval-svm verify --records results/demo-records.jsonl --line 7 --authority $AUTH
```

**Esperado:**

```
anchored batch 0: lines 0–9 (10 records) tx …
VERIFIED line 7 — batch 0, slot …, registry … (authority <sua carteira>)
```

Se aparecer `PENDING: … retry in ~15 s` (código 2), o lote ainda não está finalizado. Espere e rode de novo.

**Teste de adulteração (o momento da demo):**

```bash
cp results/demo-records.jsonl results/demo-tampered.jsonl
cp results/demo-records.jsonl.proofs.jsonl results/demo-tampered.jsonl.proofs.jsonl
sed -i '8s/deny/allow/;8s/flag/allow/' results/demo-tampered.jsonl     # muda o veredito da linha 7
./svm/target/release/aval-svm verify --records results/demo-tampered.jsonl --line 7 --authority $AUTH
# esperado: NOT VERIFIED: leaf mismatch …   (código 1)
```

No macOS, troque `sed -i` por `sed -i ''`. Se a linha 7 já for `allow`, mude qualquer outro caractere dela.

**Ancorar mais:** acrescente linhas ao `results/demo-records.jsonl` e rode o `anchor --once` de novo. Deve sair `anchored batch 1: lines 10–…`.

**Desligar:**

```bash
registry/scripts/localnet.sh --stop
```

**Códigos de saída do `verify`:**

| Código | Significado |
|---|---|
| 0 | verificado |
| 1 | NÃO verificado (adulterado, linha errada, autoridade errada) |
| 2 | não deu para checar (RPC fora, ou `PENDING`: tente de novo) |

---

## 5. Usar com os vereditos reais do VETO

O VETO grava os vereditos em JSONL só de acréscimo (`FileRecordStore`). Aponte o batcher para esse arquivo, sem `--once`, para ficar rodando:

```bash
./svm/target/release/aval-svm anchor --records /caminho/para/records.jsonl --keypair ~/.config/solana/id.json
```

Ele ancora a cada 30 s ou 256 vereditos e grava as provas em `records.jsonl.proofs.jsonl`. **Faça backup desse arquivo de provas.**

Se uma linha já ancorada for alterada, o batcher **se recusa a continuar** (código 2). É de propósito.

Para o VETO simular pelo aval-svm, aponte a URL de RPC do VETO para `http://127.0.0.1:8899` com o `aval-svm serve` rodando, e rode a suíte de testes do próprio VETO. Esse é o teste de integração de verdade.

---

## 6. Devnet (link público no explorer)

SOL de devnet é dinheiro de teste: **não custa nada real**. O único obstáculo é o limite do faucet.

- O endereço do programa (`5t75hMEMtV5rEN7BuRu3pQfyu2yUc5DVMBFqvdLxoZN5`) vem de um keypair que **só o Lucas tem**, e que não vai para o git.
- **Caminho recomendado:** o Lucas publica na devnet, e você testa contra ela.
- **Para testar contra a devnet:**
  1. Pegue ~0,1 SOL de devnet em https://faucet.solana.com (login com GitHub ajuda) para a sua carteira.
  2. Rode os mesmos comandos da seção 4 **sem** `AVAL_REGISTRY_RPC` (o padrão já é a devnet).
- **Para quem publica** (~1,1 SOL de devnet):

  ```bash
  solana program deploy --url devnet --program-id registry/target/deploy/aval_registry-keypair.json svm/tests/fixtures/aval_registry.so
  ```

---

## 7. Só se for recompilar o programa

Precisa do Anchor CLI 1.2.1 (`cargo install --git https://github.com/solana-foundation/anchor avm && avm install 1.2.1 && avm use 1.2.1`).

Ao gerar um **endereço novo**, atualize todos estes pontos, senão os testes falham:

- `declare_id!` em `registry/programs/aval_registry/src/lib.rs`
- `registry/Anchor.toml`
- `svm/tests/fixtures/aval_registry.id`
- `anchor build`, depois copie o `.so` para `svm/tests/fixtures/` e atualize o `.so.sha256`

---

## Não fazer agora

- **`solana program set-upgrade-authority … --final`:** trava o programa para sempre. Decisão para depois do hackathon.
- **Mainnet:** custa SOL de verdade, e nada aqui precisa dela.
- **Commitar keypairs:** qualquer `*.json` de carteira ou programa. O `.gitignore` já bloqueia `*-keypair.json`.

---

## Se algo der errado

| Sintoma | Causa provável |
|---|---|
| `cargo build` falha em `rustc` | `rustup update` (precisa de rustc ≥ 1.97.1) |
| `localnet.sh` não sobe | porta 8999 ocupada: `registry/scripts/localnet.sh --stop` |
| lab mostra `engine: "rpc"` | aval-svm não está rodando, ou o programa simulado não é suportado (fallback esperado) |
| `aval-svm serve` diz porta em uso | um `solana-test-validator` padrão está na 8899: use `AVAL_LISTEN=127.0.0.1:8898` e ajuste `aval_svm_url` no `config/settings.json` |
| `verify` dá `PENDING` | lote ainda não finalizado: espere ~15–20 s |
| `anchor` diz que o histórico foi alterado | o arquivo de vereditos foi editado ou truncado; isso é o sistema funcionando |

Desenho completo:

- simulador: `svm/docs/2026-10-06-aval-svm-design.md`
- registro: `svm/docs/2026-10-06-aval-registry-design.md`
