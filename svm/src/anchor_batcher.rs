//! Anchor batcher: tails an append-only records file, anchors Merkle roots of new complete
//! lines in the aval_registry program, and keeps two files next to the records file:
//! `<records>.proofs.jsonl` (one inclusion proof per anchored line) and
//! `<records>.anchor-state.json` (how far the file is anchored, plus a hash of that prefix).
//!
//! Durability order: the batch lands on chain, then the proofs are appended and fsync'd,
//! then the state is replaced atomically (temp + rename + dir fsync). A crash anywhere in
//! between leaves the chain ahead of the state; the next run finds the landed Batch account
//! by `first_record`, checks it holds the same root, and writes proofs/state as "recovered".
use crate::chain::{Chain, ChainError};
use crate::merkle::{self, Side};
use crate::registry_client::{
    anchor_batch_ix, batch_pda, decode_batch, decode_registry, init_registry_ix, registry_pda, BatchAccount, RegistryAccount,
    ERR_NON_CONTIGUOUS, ERR_UNAUTHORIZED,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Anchor's `ConstraintSeeds`: the Batch PDA we passed is not the one for `registry.next_batch`.
const ERR_CONSTRAINT_SEEDS: u32 = 2006;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub records: PathBuf,
    pub proofs: PathBuf,
    pub state: PathBuf,
}

impl Paths {
    pub fn for_records(records: &Path) -> Paths {
        let with = |suffix: &str| {
            let mut s = records.as_os_str().to_owned();
            s.push(suffix);
            PathBuf::from(s)
        };
        Paths { records: records.to_path_buf(), proofs: with(".proofs.jsonl"), state: with(".anchor-state.json") }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub registry: String,
    pub next_line: u64,
    pub anchored_bytes: u64,
    pub anchored_prefix_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofStep {
    pub side: Side,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofLine {
    pub line: u64,
    pub leaf: String,
    pub batch: u64,
    pub root: String,
    pub proof: Vec<ProofStep>,
    pub tx: String,
    pub registry: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredBatch {
    pub index: u64,
    pub first_line: u64,
    pub count: u32,
    pub root: [u8; 32],
    pub tx: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("anchored history was modified: {0}")]
    Tampered(String),
    #[error("state and chain disagree: state file says next line {state}, registry says next record {chain}")]
    StateMismatch { state: u64, chain: u64 },
    #[error("this keypair is not the registry authority (program error Unauthorized)")]
    Unauthorized,
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Reads up to `max` complete lines starting at byte `from_byte`, without their `\n`.
/// A final line with no `\n` (a writer mid-append) is never returned. A missing file reads as empty.
pub fn read_complete_lines(path: &Path, from_byte: u64, max: usize) -> io::Result<(Vec<Vec<u8>>, u64)> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((vec![], 0)),
        Err(e) => return Err(e),
    };
    let mut r = BufReader::new(f);
    r.seek(SeekFrom::Start(from_byte))?;
    let (mut lines, mut consumed) = (Vec::new(), 0u64);
    while lines.len() < max {
        let mut buf = Vec::new();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            break;
        }
        consumed += n as u64;
        buf.pop();
        lines.push(buf);
    }
    Ok((lines, consumed))
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Hashes the first `len` bytes of the records file; errors if the file is shorter.
fn hash_prefix(path: &Path, len: u64) -> Result<Sha256, BatchError> {
    let mut h = Sha256::new();
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound && len == 0 => return Ok(h),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(BatchError::Tampered(format!("{} is missing but {len} bytes of it were anchored", path.display())))
        }
        Err(e) => return Err(e.into()),
    };
    let size = f.metadata()?.len();
    if size < len {
        return Err(BatchError::Tampered(format!("{} is {size} bytes but its first {len} bytes were anchored (truncated)", path.display())));
    }
    let mut r = BufReader::new(f).take(len);
    io::copy(&mut r, &mut h)?;
    Ok(h)
}

fn fsync_dir(path: &Path) -> io::Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    File::open(dir)?.sync_all()
}

/// Atomically replaces the state file: temp file, fsync, rename, fsync the directory.
fn write_state(path: &Path, state: &State) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut f = File::create(&tmp)?;
    f.write_all(serde_json::to_string(state).map_err(|e| invalid(e.to_string()))?.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    fsync_dir(path)
}

/// Cuts the proofs file before the first entry for `line >= from_line` (and any torn last line),
/// so proofs written before a crash are not duplicated when the batch is recovered.
fn truncate_proofs_from(path: &Path, from_line: u64) -> io::Result<()> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut keep = 0usize;
    for chunk in bytes.split_inclusive(|&b| b == b'\n') {
        let complete = chunk.last() == Some(&b'\n');
        let line = serde_json::from_slice::<serde_json::Value>(chunk).ok().and_then(|v| v["line"].as_u64());
        match line {
            Some(n) if complete && n < from_line => keep += chunk.len(),
            _ => break,
        }
    }
    if keep < bytes.len() {
        let f = OpenOptions::new().write(true).open(path)?;
        f.set_len(keep as u64)?;
        f.sync_all()?;
    }
    Ok(())
}

/// Lines read from the records file for one batch.
struct Pending {
    lines: Vec<Vec<u8>>,
    leaves: Vec<[u8; 32]>,
    root: [u8; 32],
    bytes: u64,
}

impl Pending {
    fn new(lines: Vec<Vec<u8>>, bytes: u64) -> Pending {
        let leaves: Vec<[u8; 32]> = lines.iter().map(|l| merkle::leaf(l)).collect();
        let root = merkle::root(&leaves); // callers guarantee lines is non-empty
        Pending { lines, leaves, root, bytes }
    }
}

pub struct Batcher<C: Chain> {
    chain: C,
    signer: Keypair,
    paths: Paths,
    max_batch: usize,
    registry: Address,
    /// `Some` once `startup` passed; with it, a running hash of the anchored prefix.
    started: Option<(State, Sha256)>,
}

impl<C: Chain> Batcher<C> {
    pub fn new(chain: C, signer: Keypair, paths: Paths, max_batch: usize) -> Self {
        let registry = registry_pda(&signer.pubkey());
        Batcher { chain, signer, paths, max_batch: max_batch.max(1), registry, started: None }
    }

    pub fn chain(&self) -> &C {
        &self.chain
    }

    pub fn into_chain(self) -> C {
        self.chain
    }

    /// The Registry account this batcher anchors into (from the state file once started).
    pub fn registry(&self) -> Address {
        self.registry
    }

    /// Loads the state, refuses to run if the anchored prefix changed, creates the Registry
    /// if this is the first run, and refuses if the chain is behind the state.
    /// A chain ahead of the state is left to `anchor_pending`'s recovery.
    pub async fn startup(&mut self) -> Result<(), BatchError> {
        self.started = None;
        let state = match fs::read_to_string(&self.paths.state) {
            Ok(text) => serde_json::from_str::<State>(&text)
                .map_err(|e| invalid(format!("{} is not a valid state file: {e}", self.paths.state.display())))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => State {
                registry: registry_pda(&self.signer.pubkey()).to_string(),
                next_line: 0,
                anchored_bytes: 0,
                anchored_prefix_sha256: hex::encode(Sha256::digest(b"")),
            },
            Err(e) => return Err(e.into()),
        };
        let registry = Address::from_str(&state.registry).map_err(|_| invalid(format!("bad registry address in state: {}", state.registry)))?;
        let hasher = hash_prefix(&self.paths.records, state.anchored_bytes)?;
        if hex::encode(hasher.clone().finalize()) != state.anchored_prefix_sha256 {
            return Err(BatchError::Tampered(format!(
                "the first {} bytes ({} lines) of {} changed since they were anchored",
                state.anchored_bytes,
                state.next_line,
                self.paths.records.display()
            )));
        }
        self.registry = registry;
        let reg = match self.read_registry().await? {
            Some(r) => r,
            None if registry == registry_pda(&self.signer.pubkey()) => {
                match self.chain.send(vec![init_registry_ix(&self.signer.pubkey())], &self.signer).await {
                    Ok(_) => {}
                    Err(ChainError::Rejected { code: Some(ERR_UNAUTHORIZED), .. }) => return Err(BatchError::Unauthorized),
                    Err(e) => return Err(e.into()),
                }
                self.read_registry().await?.ok_or_else(|| ChainError::Unavailable("registry not visible after init".into()))?
            }
            // Someone else's registry that does not exist: we cannot create it.
            None => return Err(BatchError::Unauthorized),
        };
        if reg.next_record < state.next_line {
            return Err(BatchError::StateMismatch { state: state.next_line, chain: reg.next_record });
        }
        self.started = Some((state, hasher));
        Ok(())
    }

    /// Anchors one batch of up to `max_batch` complete pending lines (or recovers one that
    /// already landed). `None` when nothing is pending. On `Unavailable` nothing is written.
    pub async fn anchor_pending(&mut self) -> Result<Option<AnchoredBatch>, BatchError> {
        if self.started.is_none() {
            self.startup().await?;
        }
        let state = self.started.as_ref().expect("started").0.clone();
        let size = fs::metadata(&self.paths.records).map(|m| m.len()).unwrap_or(0);
        if size < state.anchored_bytes {
            return Err(BatchError::Tampered(format!("{} shrank below its {} anchored bytes", self.paths.records.display(), state.anchored_bytes)));
        }
        let reg = self.fresh_registry().await?;
        if reg.next_record != state.next_line {
            return self.recover(&state, &reg).await.map(Some);
        }
        let (lines, bytes) = read_complete_lines(&self.paths.records, state.anchored_bytes, self.max_batch)?;
        if lines.is_empty() {
            return Ok(None);
        }
        let p = Pending::new(lines, bytes);
        let ix = anchor_batch_ix(&self.signer.pubkey(), &self.registry, reg.next_batch, p.root, state.next_line, p.lines.len() as u32);
        match self.chain.send(vec![ix], &self.signer).await {
            Ok(tx) => self.commit(&state, reg.next_batch, p, tx, false).map(Some),
            Err(ChainError::Rejected { code: Some(ERR_UNAUTHORIZED), .. }) => Err(BatchError::Unauthorized),
            Err(e @ ChainError::Rejected { code: Some(ERR_NON_CONTIGUOUS | ERR_CONSTRAINT_SEEDS), .. }) => {
                // Our view of the registry was stale: a batch landed that the state doesn't know about.
                let reg = self.fresh_registry().await?;
                if reg.next_record == state.next_line {
                    return Err(e.into());
                }
                self.recover(&state, &reg).await.map(Some)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The chain is ahead of the state. Find the Batch whose `first_record` is the state's
    /// `next_line`; if it holds exactly the root of the next `count` pending lines, it is ours.
    async fn recover(&mut self, state: &State, reg: &RegistryAccount) -> Result<AnchoredBatch, BatchError> {
        let mismatch = BatchError::StateMismatch { state: state.next_line, chain: reg.next_record };
        if reg.next_record < state.next_line {
            return Err(mismatch);
        }
        let mut index = reg.next_batch;
        let batch: BatchAccount = loop {
            if index == 0 {
                return Err(mismatch);
            }
            index -= 1;
            let b = self.read_batch(index).await?;
            if b.first_record == state.next_line {
                break b;
            }
            if b.first_record < state.next_line {
                return Err(mismatch);
            }
        };
        let (lines, bytes) = read_complete_lines(&self.paths.records, state.anchored_bytes, batch.count as usize)?;
        if lines.len() != batch.count as usize || lines.is_empty() {
            return Err(mismatch);
        }
        let p = Pending::new(lines, bytes);
        if p.root != batch.root {
            return Err(mismatch);
        }
        self.commit(state, index, p, "recovered".into(), true)
    }

    /// Proofs first (appended, fsync'd), then the state (atomic replace).
    fn commit(&mut self, state: &State, index: u64, p: Pending, tx: String, recovering: bool) -> Result<AnchoredBatch, BatchError> {
        let registry = self.registry.to_string();
        let root_hex = hex::encode(p.root);
        let mut out = String::new();
        for (i, leaf) in p.leaves.iter().enumerate() {
            let proof = merkle::proof(&p.leaves, i).into_iter().map(|s| ProofStep { side: s.side, hash: hex::encode(s.hash) }).collect();
            let line = ProofLine {
                line: state.next_line + i as u64,
                leaf: hex::encode(leaf),
                batch: index,
                root: root_hex.clone(),
                proof,
                tx: tx.clone(),
                registry: registry.clone(),
            };
            out.push_str(&serde_json::to_string(&line).map_err(|e| invalid(e.to_string()))?);
            out.push('\n');
        }
        if recovering {
            truncate_proofs_from(&self.paths.proofs, state.next_line)?;
        }
        let mut f = OpenOptions::new().create(true).append(true).open(&self.paths.proofs)?;
        f.write_all(out.as_bytes())?;
        f.sync_all()?;
        drop(f);

        let mut hasher = self.started.as_ref().expect("started").1.clone();
        for l in &p.lines {
            hasher.update(l);
            hasher.update(b"\n");
        }
        let count = p.lines.len() as u32;
        let next = State {
            registry,
            next_line: state.next_line + count as u64,
            anchored_bytes: state.anchored_bytes + p.bytes,
            anchored_prefix_sha256: hex::encode(hasher.clone().finalize()),
        };
        write_state(&self.paths.state, &next)?;
        self.started = Some((next, hasher));
        Ok(AnchoredBatch { index, first_line: state.next_line, count, root: p.root, tx })
    }

    async fn read_registry(&self) -> Result<Option<RegistryAccount>, BatchError> {
        match self.chain.account(&self.registry).await? {
            None => Ok(None),
            Some(data) => decode_registry(&data)
                .map(Some)
                .ok_or_else(|| ChainError::Unavailable(format!("{} is not an aval_registry Registry account", self.registry)).into()),
        }
    }

    async fn fresh_registry(&self) -> Result<RegistryAccount, BatchError> {
        self.read_registry().await?.ok_or_else(|| ChainError::Unavailable(format!("registry {} not found", self.registry)).into())
    }

    async fn read_batch(&self, index: u64) -> Result<BatchAccount, BatchError> {
        let key = batch_pda(&self.registry, index);
        let data = self.chain.account(&key).await?.ok_or_else(|| ChainError::Unavailable(format!("batch {index} ({key}) not found")))?;
        decode_batch(&data).ok_or_else(|| ChainError::Unavailable(format!("{key} is not an aval_registry Batch account")).into())
    }
}
