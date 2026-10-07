//! The anchor batcher end to end against the real compiled aval_registry program in LiteSVM.
use aval_svm::anchor_batcher::*;
use aval_svm::chain::{Chain, ChainError, LiteSvmChain};
use aval_svm::merkle::{self, Step};
use aval_svm::registry_client::*;
use litesvm::LiteSVM;
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn chain() -> LiteSvmChain {
    let mut svm = LiteSVM::new();
    svm.add_program(program_id(), include_bytes!("fixtures/aval_registry.so")).unwrap();
    LiteSvmChain::new(svm)
}

fn funded(c: &LiteSvmChain) -> Keypair {
    let kp = Keypair::new();
    c.with_vm(|vm| vm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap());
    kp
}

fn record(i: usize) -> String {
    let digest = hex::encode(Sha256::digest(format!("message {i}").as_bytes()));
    format!(r#"{{"decision":"deny","messageDigest":"{digest}","n":{i}}}"#)
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn append_records(path: &Path, range: std::ops::Range<usize>) {
    for i in range {
        append(path, &format!("{}\n", record(i)));
    }
}

fn setup() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let records = dir.path().join("verdicts.jsonl");
    (dir, records)
}

fn proofs(paths: &Paths) -> Vec<ProofLine> {
    std::fs::read_to_string(&paths.proofs)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn state(paths: &Paths) -> State {
    serde_json::from_str(&std::fs::read_to_string(&paths.state).unwrap()).unwrap()
}

fn registry_of(c: &LiteSvmChain, reg: &Address) -> RegistryAccount {
    c.with_vm(|vm| decode_registry(&vm.get_account(reg).unwrap().data).unwrap())
}

fn batch_of(c: &LiteSvmChain, reg: &Address, index: u64) -> BatchAccount {
    c.with_vm(|vm| decode_batch(&vm.get_account(&batch_pda(reg, index)).unwrap().data).unwrap())
}

fn h32(s: &str) -> [u8; 32] {
    hex::decode(s).unwrap().try_into().unwrap()
}

/// Every proof line verifies against the root stored on chain, and its leaf is the record line's leaf.
fn assert_proofs_verify(c: &LiteSvmChain, paths: &Paths, reg: &Address) {
    let text = std::fs::read_to_string(&paths.records).unwrap();
    let lines: Vec<&str> = text.split_terminator('\n').collect();
    for p in proofs(paths) {
        let b = batch_of(c, reg, p.batch);
        assert_eq!(hex::encode(b.root), p.root);
        assert!(b.first_record <= p.line && p.line < b.first_record + b.count as u64);
        let leaf = merkle::leaf(lines[p.line as usize].as_bytes());
        assert_eq!(hex::encode(leaf), p.leaf);
        let steps: Vec<Step> = p.proof.iter().map(|s| Step { side: s.side, hash: h32(&s.hash) }).collect();
        assert!(merkle::verify(leaf, &steps, b.root), "proof for line {} fails", p.line);
        assert_eq!(p.registry, reg.to_string());
    }
}

fn snapshot(paths: &Paths) -> (Option<Vec<u8>>, Option<Vec<u8>>, Vec<u8>) {
    (std::fs::read(&paths.proofs).ok(), std::fs::read(&paths.state).ok(), std::fs::read(&paths.records).unwrap())
}

#[test]
fn paths_sit_next_to_the_records_file() {
    let p = Paths::for_records(Path::new("/x/v.jsonl"));
    assert_eq!(p.proofs, PathBuf::from("/x/v.jsonl.proofs.jsonl"));
    assert_eq!(p.state, PathBuf::from("/x/v.jsonl.anchor-state.json"));
}

#[test]
fn read_complete_lines_never_returns_a_partial_line() {
    let (_d, records) = setup();
    append(&records, "a\nbb\nccc");
    let (lines, used) = read_complete_lines(&records, 0, 10).unwrap();
    assert_eq!((lines, used), (vec![b"a".to_vec(), b"bb".to_vec()], 5));
    let (lines, used) = read_complete_lines(&records, 2, 1).unwrap();
    assert_eq!((lines, used), (vec![b"bb".to_vec()], 3));
    assert_eq!(read_complete_lines(&records, 5, 10).unwrap(), (vec![], 0));
}

#[tokio::test]
async fn first_batch_covers_all_lines_and_every_proof_verifies() {
    let c = chain();
    let kp = funded(&c);
    let reg = registry_pda(&kp.pubkey());
    let (_d, records) = setup();
    append_records(&records, 0..5);
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count), (0, 0, 5));
    let c = b.chain();
    assert_eq!(batch_of(c, &reg, 0).root, a.root);
    assert_eq!(proofs(&paths).len(), 5);
    assert_proofs_verify(c, &paths, &reg);
    let s = state(&paths);
    let bytes = std::fs::read(&records).unwrap();
    assert_eq!((s.registry, s.next_line, s.anchored_bytes), (reg.to_string(), 5, bytes.len() as u64));
    assert_eq!(s.anchored_prefix_sha256, hex::encode(Sha256::digest(&bytes)));
    assert!(b.anchor_pending().await.unwrap().is_none());
    // The state file uses exactly the spec's keys.
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths.state).unwrap()).unwrap();
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["anchoredBytes", "anchoredPrefixSha256", "nextLine", "registry"]);
    let p: serde_json::Value = serde_json::from_str(std::fs::read_to_string(&paths.proofs).unwrap().lines().next().unwrap()).unwrap();
    let mut keys: Vec<_> = p.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["batch", "leaf", "line", "proof", "registry", "root", "tx"]);
}

#[tokio::test]
async fn partial_last_line_waits_until_complete() {
    let c = chain();
    let kp = funded(&c);
    let reg = registry_pda(&kp.pubkey());
    let (_d, records) = setup();
    append_records(&records, 0..5);
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    append_records(&records, 5..8);
    let partial = record(8);
    let (head, tail) = partial.split_at(10);
    append(&records, head);
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count), (1, 5, 3));
    assert!(b.anchor_pending().await.unwrap().is_none(), "partial line must not be anchored");
    append(&records, &format!("{tail}\n"));
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count), (2, 8, 1));
    assert_eq!(proofs(&paths).len(), 9);
    assert_proofs_verify(b.chain(), &paths, &reg);
    assert_eq!(registry_of(b.chain(), &reg).next_record, 9);
}

#[tokio::test]
async fn max_batch_splits_pending_lines() {
    let c = chain();
    let kp = funded(&c);
    let reg = registry_pda(&kp.pubkey());
    let (_d, records) = setup();
    append_records(&records, 0..5);
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 2);
    b.startup().await.unwrap();
    let mut got = vec![];
    while let Some(a) = b.anchor_pending().await.unwrap() {
        got.push((a.index, a.first_line, a.count));
    }
    assert_eq!(got, [(0, 0, 2), (1, 2, 2), (2, 4, 1)]);
    assert_proofs_verify(b.chain(), &paths, &reg);
    assert_eq!(state(&paths).next_line, 5);
}

#[tokio::test]
async fn restart_resumes_without_reanchoring() {
    let c = chain();
    let kp = funded(&c);
    let kp2 = kp.insecure_clone();
    let reg = registry_pda(&kp.pubkey());
    let (_d, records) = setup();
    append_records(&records, 0..5);
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    let c = b.into_chain();
    let before = snapshot(&paths);
    let mut b = Batcher::new(c, kp2, paths.clone(), 256);
    b.startup().await.unwrap();
    assert!(b.anchor_pending().await.unwrap().is_none());
    assert_eq!(snapshot(&paths), before);
    assert_eq!(registry_of(b.chain(), &reg).next_batch, 1);
    append_records(&records, 5..7);
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count), (1, 5, 2));
    assert_proofs_verify(b.chain(), &paths, &reg);
}

async fn anchored_five() -> (tempfile::TempDir, Paths, LiteSvmChain, Keypair) {
    let c = chain();
    let kp = funded(&c);
    let kp2 = kp.insecure_clone();
    let (d, records) = setup();
    append_records(&records, 0..5);
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    (d, paths, b.into_chain(), kp2)
}

#[tokio::test]
async fn edited_anchored_line_refuses_to_start() {
    let (_d, paths, c, kp) = anchored_five().await;
    let mut bytes = std::fs::read(&paths.records).unwrap();
    let at = bytes.iter().position(|&b| b == b'd').unwrap(); // "deny" → "ceny"
    bytes[at] = b'c';
    std::fs::write(&paths.records, &bytes).unwrap();
    let before = snapshot(&paths);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    let r = b.startup().await;
    assert!(matches!(r, Err(BatchError::Tampered(_))), "{r:?}");
    assert!(matches!(b.anchor_pending().await, Err(BatchError::Tampered(_))));
    assert_eq!(snapshot(&paths), before);
}

#[tokio::test]
async fn truncated_records_refuse_to_start() {
    let (_d, paths, c, kp) = anchored_five().await;
    let bytes = std::fs::read(&paths.records).unwrap();
    std::fs::write(&paths.records, &bytes[..bytes.len() - 1]).unwrap();
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    let r = b.startup().await;
    assert!(matches!(r, Err(BatchError::Tampered(_))), "{r:?}");
}

#[tokio::test]
async fn deleted_anchored_line_refuses_to_start() {
    let (_d, paths, c, kp) = anchored_five().await;
    let text = std::fs::read_to_string(&paths.records).unwrap();
    let kept: String = text.lines().skip(1).map(|l| format!("{l}\n")).collect::<String>() + &format!("{}\n", record(99));
    std::fs::write(&paths.records, kept).unwrap();
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    assert!(matches!(b.startup().await, Err(BatchError::Tampered(_))));
}

/// Lands batch `index` for lines [first, first+n) of the records file directly through the chain,
/// as if the batcher had crashed after confirmation and before writing proofs/state.
async fn land_directly(c: &LiteSvmChain, kp: &Keypair, paths: &Paths, index: u64, first: u64, n: usize, root_override: Option<[u8; 32]>) {
    let text = std::fs::read_to_string(&paths.records).unwrap();
    let leaves: Vec<[u8; 32]> = text.lines().skip(first as usize).take(n).map(|l| merkle::leaf(l.as_bytes())).collect();
    let root = root_override.unwrap_or_else(|| merkle::root(&leaves));
    let reg = registry_pda(&kp.pubkey());
    c.send(vec![anchor_batch_ix(&kp.pubkey(), &reg, index, root, first, n as u32)], kp).await.unwrap();
}

#[tokio::test]
async fn crash_after_landing_is_recovered_and_next_batch_continues() {
    let (_d, paths, c, kp) = anchored_five().await;
    let reg = registry_pda(&kp.pubkey());
    append_records(&paths.records, 5..8);
    land_directly(&c, &kp, &paths, 1, 5, 3, None).await;
    let state_before = state(&paths);
    assert_eq!(state_before.next_line, 5);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count, a.tx.as_str()), (1, 5, 3, "recovered"));
    assert_eq!(state(&paths).next_line, 8);
    assert_eq!(proofs(&paths).len(), 8);
    assert_eq!(registry_of(b.chain(), &reg).next_batch, 2, "nothing double-anchored");
    append_records(&paths.records, 8..10);
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count), (2, 8, 2));
    assert_ne!(a.tx, "recovered");
    assert_eq!(proofs(&paths).len(), 10);
    assert_proofs_verify(b.chain(), &paths, &reg);
}

#[tokio::test]
async fn crash_recovery_drops_duplicate_proof_lines_written_before_the_crash() {
    let (_d, paths, c, kp) = anchored_five().await;
    let reg = registry_pda(&kp.pubkey());
    let kp2 = kp.insecure_clone();
    append_records(&paths.records, 5..8);
    // A full run anchors 5..8, then we roll the state back as if the crash hit between proofs and state.
    let state_before = std::fs::read(&paths.state).unwrap();
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    std::fs::write(&paths.state, state_before).unwrap();
    append(&paths.proofs, "{\"line\":7,\"le"); // and a torn write at the end
    let mut b = Batcher::new(b.into_chain(), kp2, paths.clone(), 256);
    b.startup().await.unwrap();
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!(a.tx, "recovered");
    let lines: Vec<u64> = proofs(&paths).iter().map(|p| p.line).collect();
    assert_eq!(lines, (0..8).collect::<Vec<_>>());
    assert_proofs_verify(b.chain(), &paths, &reg);
}

#[tokio::test]
async fn landed_batch_with_a_different_root_is_a_state_mismatch() {
    let (_d, paths, c, kp) = anchored_five().await;
    append_records(&paths.records, 5..8);
    land_directly(&c, &kp, &paths, 1, 5, 3, Some([9; 32])).await;
    let before = snapshot(&paths);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    let r = b.anchor_pending().await;
    assert!(matches!(r, Err(BatchError::StateMismatch { state: 5, chain: 8 })), "{r:?}");
    assert_eq!(snapshot(&paths), before);
}

#[tokio::test]
async fn chain_behind_state_is_a_state_mismatch_at_startup() {
    let (_d, paths, c, kp) = anchored_five().await;
    let mut s = state(&paths);
    s.next_line = 7;
    std::fs::write(&paths.state, serde_json::to_string(&s).unwrap()).unwrap();
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    let r = b.startup().await;
    assert!(matches!(r, Err(BatchError::StateMismatch { state: 7, chain: 5 })), "{r:?}");
}

#[tokio::test]
async fn wrong_signer_is_unauthorized_and_writes_nothing() {
    let c = chain();
    let a = funded(&c);
    let b_kp = funded(&c);
    let reg_a = registry_pda(&a.pubkey());
    c.send(vec![init_registry_ix(&a.pubkey())], &a).await.unwrap();
    let (_d, records) = setup();
    append_records(&records, 0..3);
    let paths = Paths::for_records(&records);
    let s = State { registry: reg_a.to_string(), next_line: 0, anchored_bytes: 0, anchored_prefix_sha256: hex::encode(Sha256::digest(b"")) };
    std::fs::write(&paths.state, serde_json::to_string(&s).unwrap()).unwrap();
    let before = snapshot(&paths);
    let mut b = Batcher::new(c, b_kp, paths.clone(), 256);
    b.startup().await.unwrap();
    let r = b.anchor_pending().await;
    assert!(matches!(r, Err(BatchError::Unauthorized)), "{r:?}");
    assert_eq!(snapshot(&paths), before);
    assert_eq!(registry_of(b.chain(), &reg_a).next_record, 0);
}

// ------------------------------------------------- flaky-chain wrappers

/// Lands the transaction, then reports the confirmation as unknown (RPC timeout after landing).
struct LandsThenTimesOut {
    inner: LiteSvmChain,
    armed: AtomicBool,
}

impl Chain for LandsThenTimesOut {
    async fn account(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        self.inner.account(key).await
    }
    async fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> Result<String, ChainError> {
        let sig = self.inner.send(ixs, signer).await?;
        if self.armed.swap(false, Ordering::SeqCst) {
            return Err(ChainError::Unavailable(format!("not confirmed in time ({} chars sig)", sig.len())));
        }
        Ok(sig)
    }
}

#[tokio::test]
async fn timeout_after_landing_writes_nothing_then_recovers() {
    let (_d, paths, c, kp) = anchored_five().await;
    let reg = registry_pda(&kp.pubkey());
    append_records(&paths.records, 5..8);
    let flaky = LandsThenTimesOut { inner: c, armed: AtomicBool::new(true) };
    let mut b = Batcher::new(flaky, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    let before = snapshot(&paths);
    let r = b.anchor_pending().await;
    assert!(matches!(r, Err(BatchError::Chain(ChainError::Unavailable(_)))), "{r:?}");
    assert_eq!(snapshot(&paths), before);
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count, a.tx.as_str()), (1, 5, 3, "recovered"));
    assert_eq!(registry_of(&b.chain().inner, &reg).next_batch, 2);
    assert_proofs_verify(&b.chain().inner, &paths, &reg);
}

/// Serves a stale Registry snapshot once (lagging RPC node), so the batcher sends with an old index.
struct StaleRegistryOnce {
    inner: LiteSvmChain,
    stale: std::sync::Mutex<Option<Vec<u8>>>,
    rejected: Arc<std::sync::Mutex<Option<ChainError>>>,
}

impl Chain for StaleRegistryOnce {
    async fn account(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        let stale = self.stale.lock().unwrap().take();
        match stale {
            Some(old) => Ok(Some(old)),
            None => self.inner.account(key).await,
        }
    }
    async fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> Result<String, ChainError> {
        let r = self.inner.send(ixs, signer).await;
        if let Err(e) = &r {
            *self.rejected.lock().unwrap() = Some(e.clone());
        }
        r
    }
}

#[tokio::test]
async fn stale_index_rejection_recovers_the_landed_batch() {
    let (_d, paths, c, kp) = anchored_five().await;
    let reg = registry_pda(&kp.pubkey());
    let stale = c.with_vm(|vm| vm.get_account(&reg).unwrap().data);
    append_records(&paths.records, 5..8);
    land_directly(&c, &kp, &paths, 1, 5, 3, None).await;
    let rejected = Arc::new(std::sync::Mutex::new(None));
    let flaky = StaleRegistryOnce { inner: c, stale: std::sync::Mutex::new(None), rejected: rejected.clone() };
    let mut b = Batcher::new(flaky, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    *b.chain().stale.lock().unwrap() = Some(stale);
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count, a.tx.as_str()), (1, 5, 3, "recovered"));
    let code = match rejected.lock().unwrap().clone() {
        Some(ChainError::Rejected { code, .. }) => code,
        other => panic!("expected a rejection, got {other:?}"),
    };
    assert!(code == Some(2006) || code == Some(ERR_NON_CONTIGUOUS), "{code:?}");
    assert_eq!(registry_of(&b.chain().inner, &reg).next_batch, 2);
    assert_proofs_verify(&b.chain().inner, &paths, &reg);
}

#[tokio::test]
async fn init_registry_on_first_run() {
    let c = chain();
    let kp = funded(&c);
    let reg = registry_pda(&kp.pubkey());
    let (_d, records) = setup();
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    assert!(b.anchor_pending().await.unwrap().is_none());
    let r = registry_of(b.chain(), &reg);
    assert_eq!((r.next_batch, r.next_record), (0, 0));
    assert_eq!(b.registry(), reg);
    let _ = Address::from_str(&reg.to_string()).unwrap();
}
