//! The anchor batcher end to end against the real compiled aval_registry program in LiteSVM.
use aval_svm::anchor_batcher::*;
use aval_svm::chain::{Chain, ChainError, CrossCheckChain, LiteSvmChain};
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
    assert_ne!(a.tx, "recovered", "the real signature survives in the truncated proof lines");
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
    let r = b.startup().await;
    assert!(matches!(r, Err(BatchError::Unauthorized)), "{r:?}");
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

// ------------------------------------------------- hardening (Task 6 rulings C, D, E)

#[tokio::test]
async fn wrong_signer_with_nothing_pending_is_unauthorized_at_startup() {
    let c = chain();
    let a = funded(&c);
    let b_kp = funded(&c);
    let reg_a = registry_pda(&a.pubkey());
    c.send(vec![init_registry_ix(&a.pubkey())], &a).await.unwrap();
    let (_d, records) = setup();
    let paths = Paths::for_records(&records);
    let s = State { registry: reg_a.to_string(), next_line: 0, anchored_bytes: 0, anchored_prefix_sha256: hex::encode(Sha256::digest(b"")) };
    std::fs::write(&paths.state, serde_json::to_string(&s).unwrap()).unwrap();
    let mut b = Batcher::new(c, b_kp, paths.clone(), 256);
    let r = b.startup().await;
    assert!(matches!(r, Err(BatchError::Unauthorized)), "{r:?}");
    assert!(!paths.proofs.exists());
}

#[tokio::test]
async fn crash_between_proofs_and_state_keeps_the_real_signature() {
    let (_d, paths, c, kp) = anchored_five().await;
    let kp2 = kp.insecure_clone();
    append_records(&paths.records, 5..8);
    let state_before = std::fs::read(&paths.state).unwrap();
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    let real = b.anchor_pending().await.unwrap().unwrap().tx;
    assert_ne!(real, "recovered");
    std::fs::write(&paths.state, state_before).unwrap();
    let mut b = Batcher::new(b.into_chain(), kp2, paths.clone(), 256);
    b.startup().await.unwrap();
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count), (1, 5, 3));
    assert_eq!(a.tx, real);
    let txs: Vec<String> = proofs(&paths).into_iter().filter(|p| p.line >= 5).map(|p| p.tx).collect();
    assert_eq!(txs, vec![real.clone(); 3]);
}

#[tokio::test]
async fn crlf_record_line_is_anchored_and_verified_on_its_exact_bytes() {
    let c = chain();
    let kp = funded(&c);
    let (_d, records) = setup();
    append(&records, &format!("{}\r\n{}\n", record(0), record(1)));
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    let p = &proofs(&paths)[0];
    assert_eq!(p.leaf, hex::encode(merkle::leaf(format!("{}\r", record(0)).as_bytes())));
    for line in 0..2 {
        let v = verify_line(b.chain(), &paths.records, &paths.proofs, line, None).await.unwrap();
        assert!(matches!(v, Verdict::Verified { batch: 0, .. }), "line {line}: {v:?}");
    }
}

#[tokio::test]
async fn partial_last_line_survives_a_restart_unanchored() {
    let c = chain();
    let kp = funded(&c);
    let kp2 = kp.insecure_clone();
    let (_d, records) = setup();
    append_records(&records, 0..3);
    let partial = record(3);
    let (head, tail) = partial.split_at(12);
    append(&records, head);
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    assert_eq!(b.anchor_pending().await.unwrap().unwrap().count, 3);
    let mut b = Batcher::new(b.into_chain(), kp2, paths.clone(), 256);
    b.startup().await.unwrap();
    assert!(b.anchor_pending().await.unwrap().is_none(), "partial line must not be anchored after a restart");
    let v = verify_line(b.chain(), &paths.records, &paths.proofs, 3, None).await.unwrap();
    assert_eq!(v, Verdict::NotVerified("no proof for line 3".into()));
    append(&records, &format!("{tail}\n"));
    let a = b.anchor_pending().await.unwrap().unwrap();
    assert_eq!((a.index, a.first_line, a.count), (1, 3, 1));
    let v = verify_line(b.chain(), &paths.records, &paths.proofs, 3, None).await.unwrap();
    assert!(matches!(v, Verdict::Verified { line: 3, batch: 1, .. }), "{v:?}");
}

// ------------------------------------------------- verifier

use aval_svm::verify::{registry_authority, render, rfc3339, verify_line, Verdict};

/// Two batches: lines 0..5 in batch 0, lines 5..8 in batch 1.
async fn anchored_two_batches() -> (tempfile::TempDir, Paths, LiteSvmChain, Keypair) {
    let (d, paths, c, kp) = anchored_five().await;
    let kp2 = kp.insecure_clone();
    append_records(&paths.records, 5..8);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    (d, paths, b.into_chain(), kp2)
}

fn rewrite_proofs(paths: &Paths, f: impl Fn(Vec<ProofLine>) -> Vec<ProofLine>) {
    let out: String = f(proofs(paths)).iter().map(|p| serde_json::to_string(p).unwrap() + "\n").collect();
    std::fs::write(&paths.proofs, out).unwrap();
}

fn not_verified(v: Verdict) -> String {
    match v {
        Verdict::NotVerified(m) => m,
        other => panic!("expected NOT VERIFIED, got {other:?}"),
    }
}

#[tokio::test]
async fn every_anchored_line_verifies_with_its_batch() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let reg = registry_pda(&kp.pubkey());
    let txs: Vec<String> = proofs(&paths).into_iter().map(|p| p.tx).collect();
    for line in 0..8u64 {
        let v = verify_line(&c, &paths.records, &paths.proofs, line, None).await.unwrap();
        let want_batch = if line < 5 { 0 } else { 1 };
        let b = batch_of(&c, &reg, want_batch);
        assert_eq!(
            v,
            Verdict::Verified { line, batch: want_batch, slot: b.slot, unix_timestamp: b.unix_timestamp, tx: txs[line as usize].clone(), registry: reg, authority: Some(kp.pubkey()) }
        );
        // Pinning the registry to the right authority changes nothing.
        let v2 = verify_line(&c, &paths.records, &paths.proofs, line, Some(&kp.pubkey())).await.unwrap();
        assert_eq!(v, v2);
    }
}

#[tokio::test]
async fn line_beyond_anchored_has_no_proof() {
    let (_d, paths, c, _kp) = anchored_two_batches().await;
    append_records(&paths.records, 8..9);
    let v = verify_line(&c, &paths.records, &paths.proofs, 8, None).await.unwrap();
    assert_eq!(v, Verdict::NotVerified("no proof for line 8".into()));
    let v = verify_line(&c, &paths.records, &paths.proofs, 1000, None).await.unwrap();
    assert_eq!(v, Verdict::NotVerified("no proof for line 1000".into()));
}

#[tokio::test]
async fn tampered_line_is_not_verified_and_mentions_the_leaf() {
    let (_d, paths, c, _kp) = anchored_two_batches().await;
    let text = std::fs::read_to_string(&paths.records).unwrap();
    std::fs::write(&paths.records, text.replacen("\"n\":6", "\"n\":66", 1)).unwrap();
    let m = not_verified(verify_line(&c, &paths.records, &paths.proofs, 6, None).await.unwrap());
    assert!(m.contains("leaf"), "{m}");
    // Untouched lines still verify.
    assert!(matches!(verify_line(&c, &paths.records, &paths.proofs, 5, None).await.unwrap(), Verdict::Verified { .. }));
}

#[tokio::test]
async fn proof_entry_with_a_wrong_root_is_not_verified_and_mentions_the_root() {
    let (_d, paths, c, _kp) = anchored_two_batches().await;
    rewrite_proofs(&paths, |mut ps| {
        ps[2].root = hex::encode([7u8; 32]);
        ps
    });
    let m = not_verified(verify_line(&c, &paths.records, &paths.proofs, 2, None).await.unwrap());
    assert!(m.contains("root"), "{m}");
}

#[tokio::test]
async fn proof_entry_whose_root_and_path_are_consistent_but_not_on_chain_fails() {
    // A forged single-leaf "batch": root = leaf, empty proof. Folds fine, but the chain disagrees.
    let (_d, paths, c, _kp) = anchored_two_batches().await;
    rewrite_proofs(&paths, |mut ps| {
        ps[2].root = ps[2].leaf.clone();
        ps[2].proof.clear();
        ps
    });
    let m = not_verified(verify_line(&c, &paths.records, &paths.proofs, 2, None).await.unwrap());
    assert!(m.contains("root"), "{m}");
}

#[tokio::test]
async fn proof_relabelled_to_another_position_is_not_verified() {
    // Lines 1 and 3 are byte-identical, so line 1's proof folds to the root for line 3's bytes too.
    let c = chain();
    let kp = funded(&c);
    let (_d, records) = setup();
    for i in [0, 1, 2, 1, 4] {
        append(&records, &format!("{}\n", record(i)));
    }
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    b.startup().await.unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    let c = b.into_chain();
    assert!(matches!(verify_line(&c, &records, &paths.proofs, 3, None).await.unwrap(), Verdict::Verified { line: 3, .. }));
    rewrite_proofs(&paths, |ps| {
        let mut forged = ps[1].clone();
        forged.line = 3;
        ps.into_iter().map(|p| if p.line == 3 { forged.clone() } else { p }).collect()
    });
    let m = not_verified(verify_line(&c, &records, &paths.proofs, 3, None).await.unwrap());
    assert!(m.contains("position"), "{m}");
}

#[tokio::test]
async fn line_outside_the_on_chain_batch_range_is_not_verified() {
    // Claim line 5 sits in batch 0 (which covers 0..5) with batch 0's proof for line 4's position.
    let c = chain();
    let kp = funded(&c);
    let (_d, records) = setup();
    for i in [0, 1, 2, 3, 4, 4] {
        append(&records, &format!("{}\n", record(i)));
    }
    let paths = Paths::for_records(&records);
    let mut b = Batcher::new(c, kp, paths.clone(), 5);
    b.startup().await.unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    b.anchor_pending().await.unwrap().unwrap();
    let c = b.into_chain();
    rewrite_proofs(&paths, |ps| {
        let mut forged = ps[4].clone();
        forged.line = 5;
        ps.into_iter().map(|p| if p.line == 5 { forged.clone() } else { p }).collect()
    });
    let m = not_verified(verify_line(&c, &records, &paths.proofs, 5, None).await.unwrap());
    assert!(m.contains("range") || m.contains("outside"), "{m}");
}

#[tokio::test]
async fn authority_pins_the_registry() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    // Another authority anchors the very same first batch in its own registry.
    let other = funded(&c);
    let other_reg = registry_pda(&other.pubkey());
    c.send(vec![init_registry_ix(&other.pubkey())], &other).await.unwrap();
    let root = h32(&proofs(&paths)[0].root);
    c.send(vec![anchor_batch_ix(&other.pubkey(), &other_reg, 0, root, 0, 5)], &other).await.unwrap();
    rewrite_proofs(&paths, |ps| {
        ps.into_iter()
            .map(|mut p| {
                p.registry = other_reg.to_string();
                p
            })
            .collect()
    });
    // Without --authority the proofs file's registry is trusted, and it is a real registry.
    assert!(matches!(verify_line(&c, &paths.records, &paths.proofs, 1, None).await.unwrap(), Verdict::Verified { .. }));
    // Pinned to the real authority, the swapped registry is refused.
    let m = not_verified(verify_line(&c, &paths.records, &paths.proofs, 1, Some(&kp.pubkey())).await.unwrap());
    assert!(m.contains("registry"), "{m}");
    // Without --authority, a registry with no such batch fails on chain.
    rewrite_proofs(&paths, |ps| {
        ps.into_iter()
            .map(|mut p| {
                p.registry = registry_pda(&Keypair::new().pubkey()).to_string();
                p
            })
            .collect()
    });
    let m = not_verified(verify_line(&c, &paths.records, &paths.proofs, 1, None).await.unwrap());
    assert!(m.contains("batch"), "{m}");
}

#[test]
fn rfc3339_formats_utc() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(1_791_244_800), "2026-10-06T00:00:00Z");
    assert_eq!(rfc3339(951_782_400 + 3661), "2000-02-29T01:01:01Z");
    assert_eq!(rfc3339(-1), "1969-12-31T23:59:59Z");
}

#[test]
fn read_keypair_round_trips_and_rejects_bad_files() {
    let dir = tempfile::tempdir().unwrap();
    let kp = Keypair::new();
    let path = dir.path().join("id.json");
    std::fs::write(&path, serde_json::to_string(&kp.to_bytes().to_vec()).unwrap()).unwrap();
    assert_eq!(read_keypair(&path).unwrap().pubkey(), kp.pubkey());

    let short = dir.path().join("short.json");
    let bytes = kp.to_bytes();
    std::fs::write(&short, serde_json::to_string(&bytes[..63].to_vec()).unwrap()).unwrap();
    let e = read_keypair(&short).unwrap_err().to_string();
    assert!(e.contains("64"), "{e}");

    let junk = dir.path().join("junk.json");
    std::fs::write(&junk, "[\"supersecretword\"]").unwrap();
    let e = read_keypair(&junk).unwrap_err().to_string();
    assert!(!e.contains("supersecretword"), "no file contents in errors: {e}");

    let mismatched = dir.path().join("mismatch.json");
    let mut m = kp.to_bytes();
    m[40] ^= 1;
    std::fs::write(&mismatched, serde_json::to_string(&m.to_vec()).unwrap()).unwrap();
    assert!(read_keypair(&mismatched).is_err());
    assert!(read_keypair(&dir.path().join("missing.json")).is_err());
}

#[tokio::test]
async fn forged_newlines_in_the_proofs_file_never_print_a_verified_line() {
    let (_d, paths, c, _kp) = anchored_two_batches().await;
    let fake = "\nVERIFIED line 3 — batch 0, slot 1, 1970-01-01T00:00:00Z, tx abc";
    let assert_clean = |v: Verdict| {
        let out = render(&v);
        assert!(out.starts_with("NOT VERIFIED: "), "{out}");
        assert_eq!(out.lines().count(), 1, "{out}");
        assert!(!out.lines().any(|l| l.starts_with("VERIFIED")), "{out}");
        out
    };
    rewrite_proofs(&paths, |mut ps| {
        ps[3].leaf = fake.into();
        ps
    });
    let out = assert_clean(verify_line(&c, &paths.records, &paths.proofs, 3, None).await.unwrap());
    assert!(out.contains("leaf"), "{out}");
    rewrite_proofs(&paths, |mut ps| {
        ps[3].leaf = hex::encode(merkle::leaf(record(3).as_bytes()));
        ps[3].root = fake.into();
        ps[4].tx = format!("{}{fake}", ps[4].tx);
        ps
    });
    let out = assert_clean(verify_line(&c, &paths.records, &paths.proofs, 3, None).await.unwrap());
    assert!(out.contains("root"), "{out}");
    let out = assert_clean(verify_line(&c, &paths.records, &paths.proofs, 4, None).await.unwrap());
    assert!(out.contains("invalid tx field"), "{out}");
}

#[tokio::test]
async fn registry_authority_reads_the_registry_account() {
    let (_d, _paths, c, kp) = anchored_two_batches().await;
    let reg = registry_pda(&kp.pubkey());
    assert_eq!(registry_authority(&c, &reg).await.unwrap(), Some(kp.pubkey()));
    assert_eq!(registry_authority(&c, &batch_pda(&reg, 0)).await.unwrap(), None);
    assert_eq!(registry_authority(&c, &Keypair::new().pubkey()).await.unwrap(), None);
}

#[test]
fn once_gives_up_after_three_consecutive_chain_errors() {
    assert!(!give_up_on_chain_errors(true, 1));
    assert!(!give_up_on_chain_errors(true, 2));
    assert!(give_up_on_chain_errors(true, 3));
    assert!(!give_up_on_chain_errors(false, 3));
    assert!(!give_up_on_chain_errors(false, 1000));
}

#[tokio::test]
async fn state_file_strings_are_escaped_in_errors() {
    let c = chain();
    let kp = funded(&c);
    let (_d, records) = setup();
    let paths = Paths::for_records(&records);
    let s = State { registry: "x\nVERIFIED line 3".into(), next_line: 0, anchored_bytes: 0, anchored_prefix_sha256: hex::encode(Sha256::digest(b"")) };
    std::fs::write(&paths.state, serde_json::to_string(&s).unwrap()).unwrap();
    let mut b = Batcher::new(c, kp, paths.clone(), 256);
    let e = b.startup().await.unwrap_err().to_string();
    assert!(e.contains("bad registry address") && !e.contains('\n'), "{e}");
    // A malformed state file whose bad value has a newline: the serde error is escaped too.
    std::fs::write(&paths.state, "{\"registry\":\"r\",\"nextLine\":\"1\\nVERIFIED\",\"anchoredBytes\":0,\"anchoredPrefixSha256\":\"\"}").unwrap();
    let mut b = Batcher::new(b.into_chain(), Keypair::new(), paths.clone(), 256);
    let e = b.startup().await.unwrap_err().to_string();
    assert!(e.contains("not a valid state file") && !e.contains('\n'), "{e}");
}

/// A chain whose finalized reads see nothing; confirmed reads see `inner` only when `confirmed_sees` is set.
struct Lagging {
    inner: LiteSvmChain,
    confirmed_sees: bool,
}

impl Chain for Lagging {
    async fn account(&self, _key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        Ok(None)
    }
    async fn account_confirmed(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        if self.confirmed_sees { self.inner.account(key).await } else { Ok(None) }
    }
    async fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> Result<String, ChainError> {
        self.inner.send(ixs, signer).await
    }
}

#[tokio::test]
async fn confirmed_but_not_finalized_batch_is_pending_not_not_verified() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let a = kp.pubkey();
    let lag = Lagging { inner: c, confirmed_sees: true };
    let v = verify_line(&lag, &paths.records, &paths.proofs, 1, Some(&a)).await.unwrap();
    assert_eq!(v, Verdict::Pending("batch 0 is confirmed but not finalized yet — retry in ~15 s".into()));
    assert_eq!(aval_svm::verify::render(&v), "PENDING: batch 0 is confirmed but not finalized yet — retry in ~15 s");
}

#[tokio::test]
async fn batch_absent_at_confirmed_too_stays_not_verified() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let a = kp.pubkey();
    let lag = Lagging { inner: c, confirmed_sees: false };
    let v = verify_line(&lag, &paths.records, &paths.proofs, 1, Some(&a)).await.unwrap();
    assert!(not_verified(v).contains("not found on chain"));
}

#[tokio::test]
async fn confirmed_batch_that_fails_the_checks_is_not_verified_rather_than_pending() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let a = kp.pubkey();
    // Point line 1's proof at batch 1 (a real, confirmed batch with a different root).
    rewrite_proofs(&paths, |ps| ps.into_iter().map(|mut p| { if p.line == 1 { p.batch = 1; } p }).collect());
    let lag = Lagging { inner: c, confirmed_sees: true };
    let m = not_verified(verify_line(&lag, &paths.records, &paths.proofs, 1, Some(&a)).await.unwrap());
    assert!(m.contains("root mismatch"), "{m}");
}

/// Wraps a chain and flips a byte in every account it serves, as a lying RPC would.
struct Tampered<C: Chain>(C);

impl<C: Chain> Chain for Tampered<C> {
    async fn account(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        Ok(self.0.account(key).await?.map(|mut d| { if let Some(b) = d.last_mut() { *b ^= 1; } d }))
    }
    async fn account_confirmed(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        Ok(self.0.account_confirmed(key).await?.map(|mut d| { if let Some(b) = d.last_mut() { *b ^= 1; } d }))
    }
    async fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> Result<String, ChainError> {
        self.0.send(ixs, signer).await
    }
}

/// Shares one LiteSVM between two views, so "two RPCs" see identical state.
struct Shared<'a>(&'a LiteSvmChain);

impl Chain for Shared<'_> {
    async fn account(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        self.0.account(key).await
    }
    async fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> Result<String, ChainError> {
        self.0.send(ixs, signer).await
    }
}

#[tokio::test]
async fn cross_checked_verify_passes_when_both_rpcs_agree() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let a = kp.pubkey();
    let cc = CrossCheckChain::new(Shared(&c), Shared(&c));
    let v = verify_line(&cc, &paths.records, &paths.proofs, 6, Some(&a)).await.unwrap();
    assert!(matches!(v, Verdict::Verified { .. }), "{v:?}");
}

#[tokio::test]
async fn cross_checked_verify_with_a_tampered_second_rpc_could_not_check() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let a = kp.pubkey();
    let cc = CrossCheckChain::new(Shared(&c), Tampered(Shared(&c)));
    let e = verify_line(&cc, &paths.records, &paths.proofs, 6, Some(&a)).await.expect_err("must not yield a verdict");
    assert!(e.to_string().contains("RPCs disagree"), "{e}");
}

#[tokio::test]
async fn cross_checked_verify_where_both_agree_the_evidence_fails_is_not_verified() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let a = kp.pubkey();
    rewrite_proofs(&paths, |ps| ps.into_iter().map(|mut p| { if p.line == 1 { p.batch = 1; } p }).collect());
    let cc = CrossCheckChain::new(Shared(&c), Shared(&c));
    let m = not_verified(verify_line(&cc, &paths.records, &paths.proofs, 1, Some(&a)).await.unwrap());
    assert!(m.contains("root mismatch"), "{m}");
}

/// Corrupts (or fails) reads of one account only, on the finalized and confirmed paths.
struct OneKey<C: Chain> {
    inner: C,
    key: Address,
    fail: bool,
}

impl<C: Chain> OneKey<C> {
    fn hit(&self, key: &Address, r: Result<Option<Vec<u8>>, ChainError>) -> Result<Option<Vec<u8>>, ChainError> {
        if *key != self.key {
            return r;
        }
        if self.fail {
            return Err(ChainError::Unavailable("registry read failed".into()));
        }
        Ok(r?.map(|mut d| { if let Some(b) = d.last_mut() { *b ^= 1; } d }))
    }
}

impl<C: Chain> Chain for OneKey<C> {
    async fn account(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        self.hit(key, self.inner.account(key).await)
    }
    async fn account_confirmed(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        self.hit(key, self.inner.account_confirmed(key).await)
    }
    async fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> Result<String, ChainError> {
        self.inner.send(ixs, signer).await
    }
}

#[tokio::test]
async fn registry_account_tampered_on_one_rpc_could_not_check() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let reg = registry_pda(&kp.pubkey());
    let cc = CrossCheckChain::new(Shared(&c), OneKey { inner: Shared(&c), key: reg, fail: false });
    let e = verify_line(&cc, &paths.records, &paths.proofs, 6, None).await.expect_err("must not be VERIFIED");
    assert!(e.to_string().contains("RPCs disagree"), "{e}");
}

#[tokio::test]
async fn registry_read_error_on_one_rpc_could_not_check() {
    let (_d, paths, c, kp) = anchored_two_batches().await;
    let reg = registry_pda(&kp.pubkey());
    let cc = CrossCheckChain::new(Shared(&c), OneKey { inner: Shared(&c), key: reg, fail: true });
    assert!(verify_line(&cc, &paths.records, &paths.proofs, 6, None).await.is_err());
    // Also without cross-check: an unreadable registry is "could not check", not "authority unknown".
    let solo = OneKey { inner: Shared(&c), key: reg, fail: true };
    assert!(verify_line(&solo, &paths.records, &paths.proofs, 6, None).await.is_err());
}
