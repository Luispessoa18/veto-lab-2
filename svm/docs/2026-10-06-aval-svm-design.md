# aval-svm — local Solana VM for Aval

*Design spec · 2026-10-06 · author: Lucas Oliveira*

## Why

Aval's simulation layer is the slowest step that isn't an LLM call. Today both
halves of the product simulate over the network:

- veto-lab-2 `src/unified_api.py::simulate_solana` POSTs `simulateTransaction`
  to a public RPC (`api.devnet.solana.com` by default), with a 30 s timeout.
- the VETO core engine (TypeScript) `src/normalization/rpc.ts` does the same, plus
  `getFeeForMessage`.

Every check pays a network round trip to a shared, rate-limited node, and
nothing in the reply says which state the decision was made against.

`aval-svm` runs the transaction **in-process** in a Solana VM (LiteSVM), fetching
the account state it needs from upstream RPC only when it isn't already cached.

## Goals

1. **Drop-in.** Speaks the JSON-RPC methods both repos already call. Neither
   repo changes code; each changes one URL.
2. **Fast.** p50 < 10 ms per `simulateTransaction` with a warm cache; with a
   cold cache, no slower than a single public-RPC simulation.
3. **Faithful.** On 200 real mainnet transactions, ≥ 98 % agreement with public
   RPC on `err`, `logs` and post-balances (shadow mode, below).
4. **Projections.** A lab-facing endpoint that says *what the transaction will
   do* (balance and authority deltas), not only whether it fails.
5. **Auditable.** Every reply carries a digest of the message and the slot of
   the state used. Same digest as Veto's `messageDigest`, so it can later be
   anchored on-chain.

## Non-goals (for Oct 8)

- Porting Veto's normalizer / `EffectSet` to Rust. Eric's TS normalizer stays
  the source of truth; it simply gets a faster RPC.
- Rewriting the lab's Python API (blacklist, Prompt Guard, LLM layers).
- WebSocket `accountSubscribe` freshness and the on-chain digest anchor. Both
  are designed for below but built after Oct 8.
- Being a validator, sending transactions, or holding any key.

## Architecture

One Rust binary, `aval-svm`, Axum + Tokio, crate at `veto-lab-2/svm/`.

```
 lab (python) ──┐                         ┌── upstream RPC (Helius / public)
                ├─► POST /   JSON-RPC ─┐  │        ▲
 Veto (ts)  ────┘                      │  │        │ getMultipleAccounts
                    POST /v1/project ──┤  │        │ (cache misses only)
                                       ▼  │        │
                               ┌──────────┴──┐  ┌──┴───────────┐
                               │  dispatcher │─►│ account cache│
                               └──────┬──────┘  └──────────────┘
                                      ▼
                               VM pool (N × LiteSVM, pre-warmed)
```

### Units

| Unit | Does | Depends on |
|---|---|---|
| `rpc` | Parses JSON-RPC, routes `simulateTransaction` locally, forwards every other method verbatim to upstream | `sim`, `upstream` |
| `project` | `/v1/project`: runs a simulation and builds the projection | `sim` |
| `sim` | Decodes the transaction, gathers required accounts, runs it in a pooled VM, returns pre/post account states + result | `cache`, `pool` |
| `cache` | `Pubkey → (Account, slot, fetched_at)`; batch-loads misses with one `getMultipleAccounts` (≤ 100 per call) | `upstream` |
| `pool` | N LiteSVM instances (default = cores) with builtin and common programs preloaded; checkout/checkin | — |
| `upstream` | Thin JSON-RPC client with deadlines | — |

### Account gathering (the step that decides speed)

For a transaction, the required set is:

1. The static account keys.
2. For v0 messages: the address-lookup-table accounts. Load them, resolve the
   writable/readonly indexes, then add the loaded addresses. That is a second
   batch only when the ALT itself was a miss.
3. For every invoked program owned by the upgradeable loader: its `programdata`
   account.
4. The cluster's `Rent`, `Clock` and `EpochSchedule` sysvars ride along in the
   first batch. The VM uses the cluster's Clock (time, epoch) with
   `slot = max(VM slot, cluster slot, fetch slot)`, so it never moves backwards.
   When a sysvar is absent or garbled the VM keeps its default (garbled is
   logged); without a cluster Clock the time is the wall clock. Other sysvars
   come from the VM.

If resolving the lookup tables fails (table missing, index out of range, invalid
table), the tables are fetched once more, bypassing the cache, before the
request fails: a table created or extended after it was cached still works.

All misses in one step go in a single `getMultipleAccounts`. Accounts that don't
exist upstream are cached as absent, with the same TTL.

**Freshness.** Cached entries expire after `cache_ttl_ms`, default 2 000 ms
(roughly 5 slots). Executable program accounts and upgradeable-loader
`ProgramData` accounts live longer, `program_ttl_ms` (default 60 000 ms), so an
upgrade is picked up within a minute. Each VM remembers a fingerprint of the
program accounts it holds (the data when ≤ 64 bytes, else length + first 64
bytes, which cover ProgramData's deploy slot and authority) and sets a program
again, with the program account pointing at it, when the fingerprint changes.
A request may pass `"aval": {"fresh": true}` to bypass the cache for plain
accounts (program entries still follow `program_ttl_ms`). Post-Oct-8,
`accountSubscribe` keeps hot accounts live and the TTL becomes a fallback.

**Cross-check (optional).** With `upstream_secondary_url` set, each fetch goes to both
providers and is compared. A provider more than `quorum_max_slot_gap` slots behind is
unavailable (error); equal data passes; differing data at different slots re-reads the
lagging side, up to `quorum_refetch_attempts` times (default 3), each with `minContextSlot`
= the other side's latest slot. Data still differing (or differing at equal slots) is a
disagreement: `QuorumSource::get_multiple_with_dissent` returns the primary's accounts and the
secondary's values as *dissent*, which the cache stores with the entry (and drops with it; a
disputed program is not pinned). Accounts are cross-checked at fetch time and then served
from the cache within the TTL.

**Two worlds (hot accounts no longer fail closed).** The engine classifies each disputed key:
- *strict* — signers (fee payer included), any account executable in either view, ProgramData,
  SPL Token / Token-2022 token accounts whose owner or delegate is a signer (either view), and
  address lookup tables (they decide which accounts load). Dissent here refuses with
  `upstreams disagree on <key>` (`-32005`), as before.
- *tolerated* — everything else (pools, oracles, third-party state, sysvars). The transaction is
  simulated in world P (primary accounts) and world S (primary accounts with the disputed keys
  set to the secondary's values). If either fails, that failure is the answer; if both succeed,
  world P's outcome is returned and world S's is kept as the alternate. `aval` carries
  `"worlds": 2, "divergent": <bool>` (divergent = error presence or a written account's
  post-state differs), and `/v1/project` adds `projectionAlternate`. With no dissent the output
  is unchanged. Raw reads (`getMultipleAccounts` from the cache) still fail closed on dissent.

A lying provider can only make Aval stricter, never looser — assuming at least one honest
provider: the stricter of the two worlds is reported, and dissent on what the signer controls
(or on code) refuses.

**Preloaded at boot.** System, SPL Token, Token-2022, Associated Token, Memo,
Compute Budget, Stake, Address Lookup Table, and a configurable list
(`preload_programs`, e.g. Jupiter v6, Orca Whirlpool, Raydium).

### VM pool

Simulation never commits state. A request checks out one VM, writes the
gathered accounts into it, simulates, reads post-state, and checks the VM back
in. Accounts written for request *k* are overwritten or ignored by request
*k+1*, because every account a transaction touches is written before each run.
That makes the pool lock-free per request. CPU-bound work runs on
`spawn_blocking`.

## Interfaces

### `POST /` — JSON-RPC

`simulateTransaction(tx, config)` with the config both callers send:

| Option | Supported |
|---|---|
| `encoding: base64 \| base58` | yes |
| `sigVerify: false` | yes. `true` is rejected with `-32602`: both callers simulate unsigned transactions. Precompile instructions (ed25519, secp256k1) are still verified |
| `replaceRecentBlockhash: true` | yes, and the reply includes `replacementBlockhash`. It is the local VM's blockhash, not the cluster's: never sign with it |
| `commitment` | accepted; it decides the commitment used for upstream fetches |
| `accounts: {encoding: base64, addresses}` | yes, post-state of the listed addresses. More addresses than the message's accounts (static + lookup-loaded) is `-32602 Too many accounts provided` |
| `innerInstructions: true` | yes |
| `minContextSlot` | ignored |

The reply matches the RPC shape `{context:{slot}, value:{err, logs, accounts,
unitsConsumed, returnData, innerInstructions, replacementBlockhash}}` plus one
extra top-level field that strict parsers ignore:

```json
"aval": { "digest": "<64 hex>", "stateSlot": 312345678, "stateSlotMin": 312345677,
          "cache": { "hits": 11, "misses": 2 }, "elapsedUs": 4210 }
```

`stateSlot` is the newest slot of any read; `stateSlotMin` the oldest slot among non-program
accounts (programs and ProgramData are cached longer and excluded), so equal values mean the
whole state came from one slot.

`getMultipleAccounts` with an explicit `encoding: "base64"`, no `dataSlice`/`minContextSlot` and 1–100 keys is served from the same cache as `simulateTransaction` (context slot = the newest cached slot among the requested keys), so it is usually the simulation's slot — not guaranteed: the slot covers only the requested keys, and concurrent cold misses are fetched separately. Other shapes (including an empty key list) are proxied.

All other methods (`getFeeForMessage`, `getLatestBlockhash`, `getAccountInfo`,
…) are proxied unchanged.

### `POST /v1/project`

Request: `{ "transaction": "<base64>", "fresh": false }`

Response:
```json
{
  "ok": true, "err": null, "unitsConsumed": 4123, "logs": ["…"],
  "projection": {
    "sol":    [{ "account": "…", "pre": 1000000000, "post": 994995000 }],
    "tokens": [{ "account": "…", "mint": "…", "owner": "…",
                 "pre": "5000000", "post": "0", "decimals": 6 }],
    "authority": [{ "account": "…", "field": "owner|delegate|closeAuthority",
                    "pre": "…", "post": "…" }],
    "closed": ["…"], "created": ["…"]
  },
  "aval": { "digest": "<64 hex>", "stateSlot": 312345678, "stateSlotMin": 312345677,
            "cache": { "hits": 11, "misses": 2 }, "elapsedUs": 4210 }
}
```

Token amounts are strings (u64). The projection covers every writable account
the transaction touched, and is built by diffing pre/post state. SPL Token and
Token-2022 accounts are decoded (amount, owner, delegate, close authority).

### Digest

`sha256` over the serialized **message** bytes, not the signatures, as in Veto's
`messageDigest` (`src/normalization/message.ts::messageDigestOf`): lowercase hex, no prefix, so the two values compare byte for byte.

## Integration

- **veto-lab-2:** new keys `"aval_svm_url": "http://127.0.0.1:8899"` and
  `"aval_svm_cluster": "devnet"` in `config/settings.json`. `simulate_solana`
  tries aval-svm first when the cluster matches, with its own timeout
  (`aval_svm_timeout_seconds`, default 12). On any exception (refused, timeout,
  HTTP error, bad JSON), `-32603`, `-32004`, `-32005`, or a successful-looking
  reply without the `aval` block (another server on the port, e.g.
  `solana-test-validator`, whose default port is also 8899) it falls back to
  the public RPC, so a machine without Rust keeps working. The layer's trace records `engine` and `aval`. Optional
  follow-up: call `/v1/project` and put `projection` in the trace.
- **Veto:** point its RPC URL env at `http://127.0.0.1:8899`. No code change.
- **Startup:** `iniciar_tudo.sh` / `00_INICIAR_TUDO.bat` start `aval-svm`
  alongside llama.cpp, only when the binary exists.

Config: `svm/aval-svm.toml` (or env) — `listen` (default `127.0.0.1:8899`),
`upstream_url`, `cache_ttl_ms`, `program_ttl_ms`, `pool_size`, `preload_programs`. One process per
cluster. Run two (devnet, mainnet) on different ports if both are needed.

## Errors

| Situation | Behaviour |
|---|---|
| Upstream down, all accounts cached | Simulate normally; `aval.cache.misses = 0` |
| Upstream down, misses exist | JSON-RPC error `-32005 upstream unavailable`. The lab maps it to REVIEW; Veto treats it as a failed simulate step. Never a fake success. |
| Malformed transaction | JSON-RPC `-32602`, same as Solana RPC |
| Transaction larger than 4096 bytes (also on `/v1/project`) | `-32602`. `PACKET_DATA_SIZE` is 1232 for legacy/v0; v1 may be larger, so 4096 is the cap |
| Transaction fails in the VM | Normal reply with `value.err` set, same as RPC |
| Program not loadable (e.g. a missing loader version) | `-32004 unsupported program <id>`, so the caller can fall back to public RPC |

## Verification

1. **Unit tests** (Rust): account gathering with ALTs and upgradeable programs;
   projection diff on fixture accounts; digest equals Veto's for Veto's fixtures. Veto is a private repo, so its fixtures are read from a local checkout via `VETO_FIXTURES=<path>` and never copied into this public repo; the test skips when the variable is unset.
2. **Shadow mode:** `aval-svm shadow --block <slot> --count 200` replays real
   mainnet transactions through both aval-svm and upstream `simulateTransaction`
   at the same slot and reports agreement on `err`, `logs` and post-balances.
   Target ≥ 98 %. Every disagreement is listed. Agreements are split into
   "executed and matched" and "both failed with the same error", so a run that
   agrees only on failures is visible.
3. **Benchmark:** `aval-svm shadow` also reports p50/p95 cold and warm. Run the lab's
   `09_SIMULAR_ATAQUES_SOLANA` before and after; both timings go into the
   pitch.
4. **Drop-in check:** Veto's `pnpm test` and the lab's
   `tests/test_solana_simulator.py` pass with their RPC URL pointed at aval-svm.

## Risks

- **Slot skew.** Accounts fetched at slightly different times can be
  inconsistent. Mitigation: one batch per step carries one `context.slot`, and
  the TTL is short. Shadow mode measures how much this matters.
- **LiteSVM feature gaps vs. mainnet** (feature set, newer syscalls).
  Mitigation: enable the mainnet feature set; any program error that looks like
  a gap returns `-32004` so the caller falls back.
- **Toolchain.** Rust isn't installed on the dev machine yet; installing it
  (rustup, stable) is step one of the plan.
- **Repo access.** Pushes need write access to `Luispessoa18/veto-lab-2`, or
  the work goes through a fork + PR.

## Later (after Oct 8)

- `accountSubscribe` WebSocket cache freshness.
- On-chain anchor: batch `aval.digest`s into a Merkle root posted with a
  small Anchor program or a Memo, so any verdict can be proven against a root.
- Optional native EffectSet output if Veto wants to drop RPC entirely.
