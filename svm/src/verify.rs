//! Verifier: proves one line of a records file against the batch roots stored on chain by
//! the aval_registry program. Nothing from the proofs file is trusted on its own: the leaf is
//! recomputed from the line's bytes, the proof must fold to a root the chain holds, the line
//! must sit inside that Batch's range, and the proof's shape must be the one for that position.
use crate::anchor_batcher::{BatchError, ProofLine};
use crate::chain::Chain;
use crate::merkle::{self, Step};
use crate::registry_client::{batch_pda, decode_batch, registry_pda};
use solana_address::Address;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Verified { line: u64, batch: u64, slot: u64, unix_timestamp: i64, tx: String },
    /// The first check that failed.
    NotVerified(String),
}

/// Record line `n` (0-based) without its `\n`; `None` if the file has no complete line `n`.
fn record_line(path: &Path, n: u64) -> io::Result<Option<Vec<u8>>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut r = BufReader::new(f);
    let mut buf = Vec::new();
    for i in 0..=n {
        buf.clear();
        if r.read_until(b'\n', &mut buf)? == 0 || buf.last() != Some(&b'\n') {
            return Ok(None);
        }
        if i == n {
            buf.pop();
            return Ok(Some(buf));
        }
    }
    unreachable!("the loop returns at i == n")
}

/// The first complete, parseable entry for `line` in the proofs file.
fn proof_entry(path: &Path, line: u64) -> io::Result<Option<ProofLine>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut r = BufReader::new(f);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if r.read_until(b'\n', &mut buf)? == 0 || buf.last() != Some(&b'\n') {
            return Ok(None);
        }
        if let Ok(p) = serde_json::from_slice::<ProofLine>(&buf) {
            if p.line == line {
                return Ok(Some(p));
            }
        }
    }
}

fn h32(s: &str) -> Option<[u8; 32]> {
    hex::decode(s).ok()?.try_into().ok()
}

/// Checks line `line` of `records` against the chain. `authority`, when given, pins the registry
/// (otherwise the registry named by the proofs file is used). `Err` only for I/O or an unreachable chain.
pub async fn verify_line<C: Chain>(chain: &C, records: &Path, proofs: &Path, line: u64, authority: Option<&Address>) -> Result<Verdict, BatchError> {
    let no = |m: String| Ok(Verdict::NotVerified(m));
    let Some(entry) = proof_entry(proofs, line)? else {
        return no(format!("no proof for line {line}"));
    };
    let Ok(registry) = Address::from_str(&entry.registry) else {
        return no(format!("proof entry names an invalid registry address {:?}", entry.registry));
    };
    if let Some(auth) = authority {
        let expected = registry_pda(auth);
        if registry != expected {
            return no(format!("proof entry names registry {registry}, but the registry of authority {auth} is {expected}"));
        }
    }
    let Some(bytes) = record_line(records, line)? else {
        return no(format!("record line {line} is missing from {} (no complete line)", records.display()));
    };
    let leaf = merkle::leaf(&bytes);
    if h32(&entry.leaf) != Some(leaf) {
        return no(format!("leaf mismatch: line {line} hashes to {}, the proof entry has {}", hex::encode(leaf), entry.leaf));
    }
    let Some(root) = h32(&entry.root) else {
        return no(format!("proof entry root {:?} is not 32 bytes of hex", entry.root));
    };
    let mut steps = Vec::with_capacity(entry.proof.len());
    for s in &entry.proof {
        let Some(hash) = h32(&s.hash) else {
            return no(format!("proof step hash {:?} is not 32 bytes of hex", s.hash));
        };
        steps.push(Step { side: s.side, hash });
    }
    if !merkle::verify(leaf, &steps, root) {
        return no(format!("the proof does not fold line {line}'s leaf to the entry's root {}", entry.root));
    }
    let key = batch_pda(&registry, entry.batch);
    let Some(data) = chain.account(&key).await? else {
        return no(format!("batch {} ({key}) of registry {registry} not found on chain", entry.batch));
    };
    let Some(batch) = decode_batch(&data) else {
        return no(format!("account {key} is not an aval_registry Batch"));
    };
    if batch.registry != registry || batch.index != entry.batch {
        return no(format!("on-chain batch {key} belongs to registry {} index {}, not {registry} index {}", batch.registry, batch.index, entry.batch));
    }
    if batch.root != root {
        return no(format!("root mismatch: batch {} on chain holds root {}, the proof entry has {}", entry.batch, hex::encode(batch.root), entry.root));
    }
    let end = batch.first_record + batch.count as u64;
    if line < batch.first_record || line >= end {
        return no(format!("line {line} is outside batch {}'s on-chain range {}..{end}", entry.batch, batch.first_record));
    }
    let sides: Vec<_> = steps.iter().map(|s| s.side).collect();
    if sides != merkle::expected_sides(batch.count as usize, (line - batch.first_record) as usize) {
        return no(format!("the proof's shape is not the one for line {line}'s position in batch {}", entry.batch));
    }
    Ok(Verdict::Verified { line, batch: entry.batch, slot: batch.slot, unix_timestamp: batch.unix_timestamp, tx: entry.tx })
}

/// `unix` seconds as an RFC 3339 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn rfc3339(unix: i64) -> String {
    let (days, secs) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}
