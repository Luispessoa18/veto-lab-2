//! Verifier: proves one line of a records file against the batch roots stored on chain by
//! the aval_registry program. Nothing from the proofs file is trusted on its own: the leaf is
//! recomputed from the line's bytes, the proof must fold to a root the chain holds, the line
//! must sit inside that Batch's range, and the proof's shape must be the one for that position.
use crate::anchor_batcher::{escaped, BatchError, ProofLine};
use crate::chain::Chain;
use crate::merkle::{self, Step};
use crate::registry_client::{batch_pda, decode_batch, decode_registry, registry_pda};
use solana_address::Address;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// `tx` is validated (base58 of 64 bytes, or "recovered"); `registry` is the one checked on chain
    /// and `authority` the one recorded in its Registry account (`None` if unreadable).
    Verified { line: u64, batch: u64, slot: u64, unix_timestamp: i64, tx: String, registry: Address, authority: Option<Address> },
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

/// A string from the proofs file, safe to print: canonical hex when it is a 32-byte hash,
/// else an escaped (`{:?}`) and truncated rendering, so it can never start a new output line.
fn shown(s: &str) -> String {
    match h32(s) {
        Some(h) => hex::encode(h),
        None => escaped(s),
    }
}

/// A transaction signature (base58 of 64 bytes) or the batcher's "recovered" marker.
fn valid_tx(tx: &str) -> bool {
    tx == "recovered" || bs58::decode(tx).into_vec().is_ok_and(|b| b.len() == 64)
}

/// The one line `aval-svm verify` prints. Control characters in a reason are escaped,
/// so the output is always exactly one line.
pub fn render(v: &Verdict) -> String {
    match v {
        Verdict::Verified { line, batch, slot, unix_timestamp, tx, registry, authority } => {
            let tx = if tx == "recovered" { "unknown (recovered after restart)" } else { tx.as_str() };
            let authority = authority.map_or("unknown".to_string(), |a| a.to_string());
            format!("VERIFIED line {line} — batch {batch}, slot {slot}, {}, tx {tx}, registry {registry} (authority {authority})", rfc3339(*unix_timestamp))
        }
        Verdict::NotVerified(reason) => {
            let safe: String = reason.chars().map(|c| if c.is_control() { c.escape_default().to_string() } else { c.to_string() }).collect();
            format!("NOT VERIFIED: {safe}")
        }
    }
}

/// The authority recorded in a Registry account, or `None` if the account is missing or not a Registry.
pub async fn registry_authority<C: Chain>(chain: &C, registry: &Address) -> Result<Option<Address>, BatchError> {
    Ok(chain.account(registry).await?.and_then(|d| decode_registry(&d)).map(|r| r.authority))
}

/// Checks line `line` of `records` against the chain. `authority`, when given, pins the registry
/// (otherwise the registry named by the proofs file is used). `Err` only for I/O or an unreachable chain.
pub async fn verify_line<C: Chain>(chain: &C, records: &Path, proofs: &Path, line: u64, authority: Option<&Address>) -> Result<Verdict, BatchError> {
    let no = |m: String| Ok(Verdict::NotVerified(m));
    let Some(entry) = proof_entry(proofs, line)? else {
        return no(format!("no proof for line {line}"));
    };
    let Ok(registry) = Address::from_str(&entry.registry) else {
        return no(format!("proof entry names an invalid registry address {}", shown(&entry.registry)));
    };
    if !valid_tx(&entry.tx) {
        return no(format!("invalid tx field {} in the proof entry", shown(&entry.tx)));
    }
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
        return no(format!("leaf mismatch: line {line} hashes to {}, the proof entry has {}", hex::encode(leaf), shown(&entry.leaf)));
    }
    let Some(root) = h32(&entry.root) else {
        return no(format!("proof entry root {} is not 32 bytes of hex", shown(&entry.root)));
    };
    let mut steps = Vec::with_capacity(entry.proof.len());
    for s in &entry.proof {
        let Some(hash) = h32(&s.hash) else {
            return no(format!("proof step hash {} is not 32 bytes of hex", shown(&s.hash)));
        };
        steps.push(Step { side: s.side, hash });
    }
    if !merkle::verify(leaf, &steps, root) {
        return no(format!("the proof does not fold line {line}'s leaf to the entry's root {}", hex::encode(root)));
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
        return no(format!("root mismatch: batch {} on chain holds root {}, the proof entry has {}", entry.batch, hex::encode(batch.root), hex::encode(root)));
    }
    let end = batch.first_record + batch.count as u64;
    if line < batch.first_record || line >= end {
        return no(format!("line {line} is outside batch {}'s on-chain range {}..{end}", entry.batch, batch.first_record));
    }
    let sides: Vec<_> = steps.iter().map(|s| s.side).collect();
    if sides != merkle::expected_sides(batch.count as usize, (line - batch.first_record) as usize) {
        return no(format!("the proof's shape is not the one for line {line}'s position in batch {}", entry.batch));
    }
    let authority = registry_authority(chain, &registry).await.ok().flatten();
    Ok(Verdict::Verified { line, batch: entry.batch, slot: batch.slot, unix_timestamp: batch.unix_timestamp, tx: entry.tx, registry, authority })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_never_emits_more_than_one_line() {
        let forged = Verdict::NotVerified("leaf mismatch: x\nVERIFIED line 3 — batch 0, slot 1, 1970-01-01T00:00:00Z, tx y\r\n".into());
        let out = render(&forged);
        assert_eq!(out.lines().count(), 1, "{out}");
        assert!(out.starts_with("NOT VERIFIED: "), "{out}");
        assert!(!out.lines().any(|l| l.starts_with("VERIFIED")), "{out}");
    }

    #[test]
    fn verified_line_names_the_registry_and_its_authority() {
        let (reg, auth) = (Address::from([7; 32]), Address::from([9; 32]));
        let v = |tx: &str, authority| Verdict::Verified { line: 3, batch: 1, slot: 42, unix_timestamp: 0, tx: tx.into(), registry: reg, authority };
        let sig = bs58::encode([1u8; 64]).into_string();
        assert_eq!(
            render(&v(&sig, Some(auth))),
            format!("VERIFIED line 3 — batch 1, slot 42, 1970-01-01T00:00:00Z, tx {sig}, registry {reg} (authority {auth})")
        );
        let unknown = render(&v("recovered", None));
        assert!(unknown.ends_with(&format!("tx unknown (recovered after restart), registry {reg} (authority unknown)")), "{unknown}");
    }

    #[test]
    fn shown_escapes_and_truncates_non_hash_strings() {
        let h = hex::encode([0xabu8; 32]);
        assert_eq!(shown(&h.to_uppercase()), h);
        assert_eq!(shown("a\nb"), "\"a\\nb\"");
        assert!(shown(&"x".repeat(500)).len() < 100);
    }

    #[test]
    fn tx_must_be_a_signature_or_recovered() {
        assert!(valid_tx("recovered"));
        assert!(valid_tx(&bs58::encode([1u8; 64]).into_string()));
        assert!(!valid_tx(&bs58::encode([1u8; 63]).into_string()));
        assert!(!valid_tx("abc\nVERIFIED"));
        assert!(!valid_tx(""));
    }
}
