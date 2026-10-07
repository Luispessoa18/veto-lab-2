# aval-registry — on-chain attestation of Aval verdicts

*Design spec · 2026-10-06 · author: Lucas Oliveira · builds on `2026-10-06-aval-svm-design.md`*

## Why

Aval's claim is transparency: anyone should be able to check what Aval decided
about a transaction, and that the decision was not edited afterwards. Today
the VETO engine writes every verdict to an append-only JSONL file
(`FileRecordStore`, one `ActionRecord` per line: `messageDigest`, `decision`,
`policyVersion`, `findings`, time…). Append-only on a disk the operator controls
is a promise, not evidence.

The team's closed technical decisions approve exactly one on-chain Rust
component: a registry program that attests evaluations — a tamper-evident log
with **no authority over value and never custody**. This spec is that component.

## Goals

1. Every verdict line written by the VETO engine is committed on-chain within
   ~30 s, at a fixed cost per batch rather than per verdict.
2. Anyone with the record line, its proof, and an RPC endpoint can verify it,
   without trusting the operator's disk or database.
3. Rewriting or deleting an already-anchored line is detected.
4. Zero changes to the VETO engine: the batcher reads its JSONL file.
5. Running on devnet for the demo by Oct 8.

## Non-goals

- Mainnet deployment, a hosted batcher, or key management beyond a keypair file.
- A "verified" badge in the VETO panel (follow-up: it would read the proofs file).
- Attesting aval-svm simulations; the verdict is what is attested.
- Any instruction that moves lamports other than paying rent for new accounts.

## Components

```
 VETO engine ──append──► records.jsonl ──tail──► aval-svm anchor ──anchor_batch tx──► aval_registry (devnet)
                                                     │                                      │
                                                     └──► records.proofs.jsonl              │
                                                                                            ▼
 anyone: aval-svm verify --line N  ── reads line + proof, recomputes root, reads Batch account
```

| Unit | Location | Does |
|---|---|---|
| `aval_registry` program | `registry/programs/aval_registry` (Anchor 1.x) | Stores batch roots in PDAs, enforces ordering and authority |
| `merkle` | `svm/src/merkle.rs` | Leaf/inner hashing, tree, proofs, verification (pure) |
| `registry_client` | `svm/src/registry_client.rs` | PDA derivation, instruction encoding, account decoding (pure) |
| `anchor` batcher | `svm/src/anchor_batcher.rs` + CLI `aval-svm anchor` | Tails the file, batches, submits, writes proofs + state |
| `verify` | `svm/src/verify.rs` + CLI `aval-svm verify` | Recomputes and checks a line against the chain |

The svm crate does **not** depend on `anchor-lang` (its Solana crate versions may
not match litesvm's). It encodes the two instructions and decodes the two
accounts by hand — Anchor's wire format is stable and small — and tests that
encoding against the real compiled program in LiteSVM.

## On-chain program `aval_registry`

Accounts:

| Account | Seeds | Fields |
|---|---|---|
| `Registry` | `["registry", authority]` | `authority: Pubkey`, `next_batch: u64`, `next_record: u64`, `last_root: [u8;32]`, `bump: u8` |
| `Batch` | `["batch", registry, next_batch.to_le_bytes()]` | `registry: Pubkey`, `index: u64`, `root: [u8;32]`, `first_record: u64`, `count: u32`, `slot: u64`, `unix_timestamp: i64`, `bump: u8` |

Instructions:

- `init_registry()` — signer = authority = payer. Creates `Registry` with zeros.
- `anchor_batch(root: [u8;32], first_record: u64, count: u32)` — signer must be
  `registry.authority`. Requires `count >= 1`, `first_record == registry.next_record`
  (batches are contiguous: no gaps, no overlap, no replay). Creates the `Batch`
  PDA for `registry.next_batch`, stores root/first_record/count and the current
  `Clock` slot + unix_timestamp, then sets `next_batch += 1`,
  `next_record += count`, `last_root = root`.

There is no update, close, or authority-transfer instruction. Errors:
`Unauthorized`, `NonContiguous`, `EmptyBatch` (Anchor custom errors).

## Hashing

- A record is the exact bytes of one line of the JSONL file, **without** the
  trailing `\n`. Lines are numbered from 0. A trailing line without `\n` is
  incomplete and is not read until its newline arrives.
- `leaf = sha256(0x00 ‖ line_bytes)`; `node = sha256(0x01 ‖ left ‖ right)`.
  The prefixes make a leaf impossible to pass off as an inner node.
- Odd levels: the unpaired last node is promoted unchanged to the next level
  (never duplicated — duplication lets two different lists share a root).
- One leaf → root = that leaf.
- A proof is the list of `(side, hash)` siblings from leaf to root; `side`
  says whether the sibling is on the left or right.

## Batcher `aval-svm anchor`

```
aval-svm anchor --records <records.jsonl> --keypair <file> [--upstream <rpc>]
                [--interval-secs 30] [--max-batch 256] [--once]
```

- Proofs file: `<records>.proofs.jsonl`, one line per anchored record:
  `{"line":n,"leaf":hex,"batch":k,"root":hex,"proof":[{"side":"L"|"R","hash":hex}],"tx":sig,"registry":pubkey}`.
- State file: `<records>.anchor-state.json`:
  `{"registry":pubkey,"nextLine":n,"anchoredBytes":b,"anchoredPrefixSha256":hex}`.
- Loop: read complete lines from byte offset `anchoredBytes`; when
  `--max-batch` lines are pending or `--interval-secs` elapsed with ≥1 pending,
  build the tree, send `anchor_batch(root, nextLine, count)`, wait for
  `confirmed`, then append the proofs and advance the state (proofs first,
  state second, both fsync'd). `--once` anchors what is pending and exits.
- First run: sends `init_registry` if the Registry PDA doesn't exist.
- Restart: recompute sha256 of the file's first `anchoredBytes` bytes; if it
  differs from `anchoredPrefixSha256`, or the file is shorter, **refuse to run**
  and print that anchored history was modified. Then compare the on-chain
  `registry.next_record` with `nextLine`:
  - chain **behind** the state: refuse to run (state and chain disagree),
    printing both numbers;
  - chain **ahead** of the state (a batch landed but the state write was lost,
    e.g. a crash): recover rather than refuse. Search the Batch accounts
    backwards from `registry.next_batch - 1` for the one whose
    `first_record == nextLine`, read the next `count` complete lines, and
    compare their Merkle root with the Batch's root. If root and count match,
    the batch is ours: write its proofs and advance the state, keeping the real
    transaction signature when a proof line from before the crash still holds
    it (otherwise the `tx` field is `"recovered"`). If they do not match, refuse
    and print both numbers.
- Transport: blockhash via `getLatestBlockhash`, signed legacy transaction,
  `sendTransaction` (base64), `getSignatureStatuses` until `confirmed` or the
  blockhash expires. An `Unavailable` outcome (RPC down, blockhash expired, not
  confirmed in time) leaves nothing written; the next tick re-reads the Registry
  and decides again, so a transaction that landed late is found by that read
  rather than resent blindly. A `NonContiguous` rejection (6001) or a seeds
  constraint failure (2006, the Batch PDA for a stale `next_batch`) means our
  view was stale: the batcher re-reads the Registry and, if the chain is ahead,
  runs the recovery above; if the chain still matches the state, the error is
  surfaced.
- `--once` anchors what is pending and exits; after 3 consecutive chain errors
  it gives up and exits with code 2. The long-running mode retries forever.
- The keypair is read from a file path, never printed, never written anywhere.
  The batcher derives the Registry PDA from the keypair and refuses at startup,
  client-side, if the Registry's recorded authority is not that keypair.

## Verifier `aval-svm verify`

```
aval-svm verify --records <records.jsonl> --line N [--proofs <file>] [--upstream <rpc>]
                [--authority <pubkey>]
```

Reads line N, its proof entry, recomputes leaf → root, derives the Batch PDA,
fetches it over RPC at `finalized` commitment (the batcher itself uses
`confirmed`), and checks: root equal, `first_record ≤ N < first_record+count`,
registry matches. Nothing in the proofs file is trusted on its own:
the leaf is recomputed from the line's bytes, and the proof's shape must be the
one for line N's position in the batch (`expected_sides` for `count` and
`N − first_record`), so a valid proof for another position cannot be passed off.

`--authority` pins the registry: the registry must be the PDA of that authority,
instead of whatever the proofs file names. Without it, the registry comes from the
proofs file and a note on stderr says so.

Output: `VERIFIED line N — batch k, slot S, <RFC 3339 time>, tx <sig|unknown (recovered after restart)>, registry <pda> (authority <pubkey|unknown>)`
(authority read from the on-chain Registry account) or `NOT VERIFIED: <the first check that failed>`.
Anything taken from the proofs file that is printed is hex-canonicalised or
escaped and truncated, so the output is always exactly one line and a forged file
cannot print a fake `VERIFIED`.

Exit codes: `0` verified; `1` not verified; `2` could not check (I/O, RPC
unreachable, bad arguments).
`POST /v1/verify` on the server is optional and out of scope for Oct 8.

## Errors

| Situation | Behaviour |
|---|---|
| RPC down / tx not confirmed | batch stays pending, retried next tick (the Registry is re-read first); nothing written to proofs/state. `--once` exits 2 after 3 in a row |
| Anchored prefix modified or file truncated | refuse to run, explain, exit non-zero |
| State vs chain: chain behind the state | refuse to run, print both counters |
| State vs chain: chain ahead of the state | recover the landed batch if root and count match the pending lines; otherwise refuse and print both counters |
| Malformed JSON line | anchored as bytes anyway (the attestation is of what was written, not of its validity) |
| Wrong keypair (not the registry authority) | refused at startup, client-side, before any transaction is sent (the program would also reject `Unauthorized`) |

## Testing

1. `merkle`: known vectors (1, 2, 3, 5, 8 leaves), proof round-trip for every
   leaf, tampered leaf / swapped sibling / wrong side fail, leaf-vs-inner
   prefix separation.
2. Program in LiteSVM (compiled `.so` committed at
   `svm/tests/fixtures/aval_registry.so`, pinned by a committed sha256 file that
   a test checks the `.so` against): init; two contiguous
   batches; gap → `NonContiguous`; replay → `NonContiguous`; zero count →
   `EmptyBatch`; wrong signer → `Unauthorized`; account fields decode as written.
3. Batcher + verifier end to end in LiteSVM through a test RPC shim or a
   transport trait: write lines, `--once`, verify every line, append more,
   anchor again (contiguity), tamper an anchored line → batcher refuses and
   verify fails.
4. Devnet: deploy, anchor a real VETO records file, verify one line; record
   program id and an explorer link in the README.

## Toolchain

Solana CLI (Agave 4.x) and Anchor CLI 1.x (via `avm`) are required only to
build/deploy the program. The svm crate and its tests do not need them: they
use the committed `.so`.

## Risks

- Anchor 1.x wire-format details (discriminators, account layout) — mitigated by
  encoding tests against the real compiled program.
- Devnet airdrop limits for deploy rent (~1–2 SOL for the program) — fallback:
  faucet.solana.com or a teammate's devnet SOL.
- A batcher operator can leave the tail of the file unanchored, or alter lines
  that have not been anchored yet. Neither is visible on chain until the lines
  are anchored; it is detectable only by comparing the records file with the
  proofs file. Gaps in the middle are impossible: batches are contiguous, so
  anything anchored is anchored in order and an anchored line cannot change. Out
  of scope for Oct 8.
