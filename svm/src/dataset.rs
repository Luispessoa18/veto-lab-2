//! `aval-svm dataset`: samples real transactions from recent finalized blocks, simulates each one
//! fresh against current state, and appends the effects of the successful ones (those that touch
//! a signer beyond the fee) as JSON lines for the lab's training-dataset builder.
use crate::cache::Cache;
use crate::decode::{decode, Encoding};
use crate::engine::{Engine, EngineError, SimReport};
use crate::pool::Pool;
use crate::project::{project, token_mint_owner, Projection};
use crate::shadow::VOTE_PROGRAM;
use crate::source::SourceError;
use crate::upstream::Upstream;
use serde::Serialize;
use serde_json::{json, Value};
use solana_account::Account;
use solana_address::Address;
use solana_transaction::versioned::VersionedTransaction;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Who signs the transaction and which programs its top-level instructions call.
#[derive(Debug, Clone, PartialEq)]
pub struct TxMeta {
    pub signers: Vec<String>,
    pub fee_payer: String,
    pub programs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TokenAccount {
    pub mint: String,
    pub owner: String,
}

/// One JSON line of the effects file.
#[derive(Debug, Serialize)]
pub struct EffectRecord {
    pub tx_digest: String,
    /// Slot of the block the transaction was sampled from.
    pub slot: u64,
    /// Slot of the state it was simulated against.
    pub state_slot: u64,
    pub signers: Vec<String>,
    pub fee_payer: String,
    pub programs: Vec<String>,
    pub projection: Projection,
    pub units: u64,
    pub fee: u64,
    /// Mint and owner of the token accounts named in `projection.authority` / `projection.closed`.
    pub token_accounts: BTreeMap<String, TokenAccount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx: Option<String>,
}

pub fn tx_meta(tx: &VersionedTransaction) -> TxMeta {
    let keys = tx.message.static_account_keys();
    let n = usize::from(tx.message.header().num_required_signatures).min(keys.len());
    let mut programs: Vec<String> = Vec::new();
    for ix in tx.message.instructions() {
        if let Some(p) = keys.get(usize::from(ix.program_id_index)).map(|k| k.to_string()) {
            if !programs.contains(&p) { programs.push(p); }
        }
    }
    TxMeta {
        signers: keys[..n].iter().map(|k| k.to_string()).collect(),
        fee_payer: keys.first().map(|k| k.to_string()).unwrap_or_default(),
        programs,
    }
}

pub fn is_vote(tx: &VersionedTransaction) -> bool {
    tx.message.static_account_keys().iter().any(|k| k.to_string() == VOTE_PROGRAM)
}

/// `top` followed by every program named in a `Program <id> invoke [n]` log line (CPIs too), once each.
pub fn invoked_programs(top: &[String], logs: &[String]) -> Vec<String> {
    let mut out: Vec<String> = top.to_vec();
    for line in logs {
        let parts: Vec<&str> = line.split(' ').collect();
        if parts.len() == 4 && parts[0] == "Program" && parts[2] == "invoke" && parts[3].starts_with('[')
            && !out.iter().any(|p| p == parts[1])
        {
            out.push(parts[1].to_string());
        }
    }
    out
}

/// Mint/owner of each token account the projection changes authority on or closes.
pub fn token_accounts(p: &Projection, pre: &HashMap<Address, Option<Account>>, post: &HashMap<Address, Account>) -> BTreeMap<String, TokenAccount> {
    let named = p.authority.iter().map(|a| &a.account).chain(&p.closed);
    let mut out = BTreeMap::new();
    for acct in named {
        let Ok(k) = acct.parse::<Address>() else { continue };
        // The state before the transaction says who owned it (an owner change or a close erases that).
        let found = pre.get(&k).and_then(|a| a.as_ref()).and_then(token_mint_owner)
            .or_else(|| post.get(&k).and_then(token_mint_owner));
        if let Some((mint, owner)) = found {
            out.insert(acct.clone(), TokenAccount { mint, owner });
        }
    }
    out
}

/// True when the effects touch a signer (or a token account a signer owns) beyond paying the fee.
pub fn touches_signers(p: &Projection, meta: &TxMeta, fee: u64, tokens: &BTreeMap<String, TokenAccount>) -> bool {
    let is_signer = |a: &str| meta.signers.iter().any(|s| s == a);
    let signer_owned = |a: &str| is_signer(a) || tokens.get(a).is_some_and(|t| is_signer(&t.owner));
    p.sol.iter().any(|d| is_signer(&d.account) && !(d.account == meta.fee_payer && d.pre.checked_sub(d.post) == Some(fee)))
        || p.tokens.iter().any(|t| is_signer(&t.owner) || is_signer(&t.account))
        || p.authority.iter().any(|a| signer_owned(&a.account))
        || p.closed.iter().any(|a| signer_owned(a))
}

/// The line to keep for a simulation, or `None` when it failed or did not touch a signer.
pub fn build_record(report: &SimReport, meta: TxMeta, block_slot: u64, tx_b64: Option<&str>) -> Option<EffectRecord> {
    let o = &report.outcome;
    if o.err.is_some() { return None; }
    let projection = project(&report.pre, &o.post);
    let tokens = token_accounts(&projection, &report.pre, &o.post);
    if !touches_signers(&projection, &meta, o.fee, &tokens) { return None; }
    Some(EffectRecord {
        tx_digest: report.digest.clone(),
        slot: block_slot,
        state_slot: report.slot,
        programs: invoked_programs(&meta.programs, &o.logs),
        signers: meta.signers,
        fee_payer: meta.fee_payer,
        projection,
        units: o.units,
        fee: o.fee,
        token_accounts: tokens,
        tx: tx_b64.map(String::from),
    })
}

/// `tx_digest`s already in the output file (missing file = none; malformed lines are ignored).
pub fn seen_digests(path: &Path) -> std::io::Result<HashSet<String>> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(e) => return Err(e),
    };
    let mut out = HashSet::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            if let Some(d) = v["tx_digest"].as_str() { out.insert(d.to_string()); }
        }
    }
    Ok(out)
}

/// Opens the output for appending; a truncated last line (no trailing newline) is terminated first.
pub fn open_append(path: &Path) -> std::io::Result<std::fs::File> {
    let mut f = std::fs::OpenOptions::new().create(true).read(true).append(true).open(path)?;
    if f.metadata()?.len() > 0 {
        let mut last = [0u8; 1];
        f.seek(SeekFrom::End(-1))?;
        f.read_exact(&mut last)?;
        if last[0] != b'\n' { f.write_all(b"\n")?; }
    }
    Ok(f)
}

/// `k` indices spread evenly over `0..n` (all of them when `k >= n`).
pub fn sample_indices(n: usize, k: usize) -> Vec<usize> {
    if k >= n { return (0..n).collect(); }
    (0..k).map(|i| i * n / k).collect()
}

#[derive(Debug, PartialEq)]
pub enum BlockFailure {
    /// The slot has no block (skipped, pruned, unsupported): move on.
    Skip,
    /// Rate limit or transport trouble: back off and try again.
    Retry,
}

pub fn block_failure(e: &SourceError) -> BlockFailure {
    match e {
        SourceError::Unavailable(_) => BlockFailure::Retry,
        SourceError::Rpc(v) => match v["code"].as_i64() {
            Some(429) | Some(-32005) | Some(-32603) => BlockFailure::Retry,
            _ => BlockFailure::Skip,
        },
    }
}

/// 1 s, 2 s, 4 s, 8 s, then 16 s.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64 << attempt.min(4))
}

/// Host of a URL only, so API keys in the path or query are never printed.
pub fn host_only(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_else(|| "<invalid url>".into())
}

#[derive(Debug, Default)]
pub struct Stats {
    pub blocks: usize,
    pub scanned: usize,
    pub kept: usize,
    pub failed: usize,
    pub no_effect: usize,
    pub errored: usize,
    pub duplicate: usize,
    pub block_errors: usize,
}

pub fn progress(slot: u64, s: &Stats, target: usize) -> String {
    format!("slot {slot}: kept {}/{target} — scanned {} (failed {}, no signer effect {}, errored {}) — blocks {} (block errors {}, duplicates skipped {})",
        s.kept, s.scanned, s.failed, s.no_effect, s.errored, s.blocks, s.block_errors, s.duplicate)
}

pub struct Args {
    pub upstream: String,
    pub target: usize,
    pub out: PathBuf,
    pub delay_ms: u64,
    pub max_blocks: usize,
    pub per_block: usize,
    pub with_tx: bool,
}

const RETRIES: u32 = 4;

async fn call_with_retry(up: &Upstream, method: &str, params: Value) -> Result<Value, SourceError> {
    let mut attempt = 0;
    loop {
        match up.call(method, params.clone()).await {
            Err(e) if block_failure(&e) == BlockFailure::Retry && attempt < RETRIES => {
                eprintln!("{method}: {} — retrying in {:?}", short(&e.to_string()), backoff(attempt));
                tokio::time::sleep(backoff(attempt)).await;
                attempt += 1;
            }
            r => return r,
        }
    }
}

async fn simulate_with_retry(engine: &Engine<Upstream>, b64: &str) -> Result<SimReport, EngineError> {
    let mut attempt = 0;
    loop {
        let d = decode(b64, Encoding::Base64).map_err(|e| EngineError::Internal(e.to_string()))?;
        match engine.simulate(d, true).await {
            Err(EngineError::Upstream(m)) if attempt < RETRIES => {
                eprintln!("simulate: upstream {} — retrying in {:?}", short(&m), backoff(attempt));
                tokio::time::sleep(backoff(attempt)).await;
                attempt += 1;
            }
            r => return r,
        }
    }
}

/// Error text can echo the request URL; keep it short and never print a URL.
fn short(s: &str) -> String {
    let cleaned: Vec<&str> = s.split_whitespace().map(|w| if w.contains("://") { "<url>" } else { w }).collect();
    cleaned.join(" ").chars().take(160).collect()
}

pub async fn run(a: &Args) -> anyhow::Result<()> {
    let mut seen = seen_digests(&a.out)?;
    let mut st = Stats { kept: seen.len(), ..Stats::default() };
    println!("dataset: upstream {} → {} ({} already kept, target {})", host_only(&a.upstream), a.out.display(), st.kept, a.target);
    if st.kept >= a.target {
        println!("target already reached");
        return Ok(());
    }
    if let Some(dir) = a.out.parent() { std::fs::create_dir_all(dir)?; }
    let mut file = open_append(&a.out)?;
    let up = Upstream::new(&a.upstream, "confirmed", 30_000);
    let cache = Cache::new(up.clone(), Duration::from_millis(2000)).with_program_ttl(Duration::from_secs(600));
    let engine = Engine::new(cache, Pool::new(4, 5000));
    let mut slot = call_with_retry(&up, "getSlot", json!([{"commitment": "finalized"}])).await?
        .as_u64().ok_or_else(|| anyhow::anyhow!("getSlot returned no slot"))?;
    let mut tried = 0usize;
    while st.kept < a.target && st.blocks < a.max_blocks && tried < a.max_blocks * 4 {
        tried += 1;
        let s = slot;
        slot = slot.saturating_sub(1);
        let block = call_with_retry(&up, "getBlock", json!([s, {"encoding": "base64", "transactionDetails": "full",
            "maxSupportedTransactionVersion": 0, "rewards": false, "commitment": "finalized"}])).await;
        let block = match block {
            Ok(b) if b.is_null() => continue,
            Ok(b) => b,
            // A slot with no block is skipped silently; one that kept failing is counted.
            Err(e) => {
                if block_failure(&e) == BlockFailure::Retry {
                    st.block_errors += 1;
                    eprintln!("slot {s}: giving up on block: {}", short(&e.to_string()));
                }
                continue;
            }
        };
        st.blocks += 1;
        let txs: Vec<String> = block["transactions"].as_array().map(|v| v.iter()
            .filter(|t| t["meta"]["err"].is_null())
            .filter_map(|t| t["transaction"][0].as_str().map(String::from))
            .filter(|b64| decode(b64, Encoding::Base64).is_ok_and(|d| !is_vote(&d.tx)))
            .collect()).unwrap_or_default();
        for i in sample_indices(txs.len(), a.per_block) {
            if st.kept >= a.target { break; }
            let b64 = &txs[i];
            let Ok(d) = decode(b64, Encoding::Base64) else { continue };
            if seen.contains(&d.digest) { st.duplicate += 1; continue; }
            let meta = tx_meta(&d.tx);
            tokio::time::sleep(Duration::from_millis(a.delay_ms)).await;
            st.scanned += 1;
            match simulate_with_retry(&engine, b64).await {
                Ok(report) => {
                    let failed = report.outcome.err.is_some();
                    match build_record(&report, meta, s, a.with_tx.then_some(b64.as_str())) {
                        Some(rec) => {
                            writeln!(file, "{}", serde_json::to_string(&rec)?)?;
                            file.flush()?;
                            seen.insert(rec.tx_digest);
                            st.kept += 1;
                        }
                        None if failed => st.failed += 1,
                        None => st.no_effect += 1,
                    }
                }
                Err(EngineError::Upstream(_)) => st.errored += 1,
                Err(_) => st.failed += 1,
            }
        }
        println!("{}", progress(s, &st, a.target));
    }
    println!("done: {}", progress(slot + 1, &st, a.target));
    println!("details: {}", a.out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::SimOutcome;
    use crate::project::{AuthorityChange, SolDelta, TokenDelta, TOKEN_PROGRAM};
    use solana_hash::Hash;
    use solana_instruction::{AccountMeta, Instruction};
    use solana_message::{Message, VersionedMessage};
    use std::str::FromStr;

    fn key(n: u8) -> Address { Address::from([n; 32]) }
    fn s(n: u8) -> String { key(n).to_string() }

    fn tx(ixs: &[Instruction], payer: Address) -> VersionedTransaction {
        let msg = Message::new_with_blockhash(ixs, Some(&payer), &Hash::new_from_array([7; 32]));
        let n = msg.header.num_required_signatures as usize;
        VersionedTransaction { signatures: vec![Default::default(); n], message: VersionedMessage::Legacy(msg) }
    }

    fn token_account(mint: Address, owner: Address) -> Account {
        let mut d = vec![0u8; 165];
        d[0..32].copy_from_slice(mint.as_ref());
        d[32..64].copy_from_slice(owner.as_ref());
        d[108] = 1;
        Account { lamports: 2_039_280, data: d, owner: Address::from_str(TOKEN_PROGRAM).unwrap(), executable: false, rent_epoch: 0 }
    }

    fn meta() -> TxMeta { TxMeta { signers: vec![s(1), s(2)], fee_payer: s(1), programs: vec![] } }

    fn report(err: Option<solana_transaction_error::TransactionError>, pre: Vec<(Address, Option<Account>)>, post: Vec<(Address, Account)>) -> SimReport {
        SimReport {
            outcome: SimOutcome {
                err, logs: vec![format!("Program {} invoke [1]", s(50)), format!("Program {} invoke [2]", s(51))],
                units: 1234, fee: 5000, inner: vec![], return_data: Default::default(),
                post: post.into_iter().collect(), blockhash: Hash::default(), clock: Default::default(),
            },
            pre: pre.into_iter().collect(), slot: 900, min_slot: 900, hits: 0, misses: 0, elapsed_us: 0, digest: "ab".repeat(32),
        }
    }

    #[test]
    fn meta_lists_signers_payer_and_top_level_programs_once() {
        let ix = |p: u8, signer: u8| Instruction::new_with_bytes(key(p), &[], vec![AccountMeta::new(key(signer), true), AccountMeta::new(key(9), false)]);
        let t = tx(&[ix(40, 1), ix(41, 2), ix(40, 1)], key(1));
        let m = tx_meta(&t);
        assert_eq!(m.fee_payer, s(1));
        assert_eq!(m.signers, vec![s(1), s(2)]);
        assert_eq!(m.programs, vec![s(40), s(41)]);
        assert!(!is_vote(&t));
        let vote = Instruction::new_with_bytes(Address::from_str(VOTE_PROGRAM).unwrap(), &[], vec![AccountMeta::new(key(1), true)]);
        assert!(is_vote(&tx(&[vote], key(1))));
    }

    #[test]
    fn invoked_programs_adds_cpis_from_logs_in_order() {
        let logs = vec![
            format!("Program {} invoke [1]", s(40)),
            format!("Program {} invoke [2]", s(42)),
            "Program log: Instruction: Transfer".to_string(),
            format!("Program {} consumed 10 of 200000 compute units", s(42)),
            format!("Program {} invoke [2]", s(42)),
            format!("Program {} success", s(40)),
        ];
        assert_eq!(invoked_programs(&[s(40), s(41)], &logs), vec![s(40), s(41), s(42)]);
    }

    #[test]
    fn fee_alone_is_not_a_signer_effect() {
        let fee_only = Projection { sol: vec![SolDelta { account: s(1), pre: 10_000, post: 5_000 }], ..Default::default() };
        assert!(!touches_signers(&fee_only, &meta(), 5000, &BTreeMap::new()));
        let more = Projection { sol: vec![SolDelta { account: s(1), pre: 10_000, post: 4_000 }], ..Default::default() };
        assert!(touches_signers(&more, &meta(), 5000, &BTreeMap::new()));
        let other_signer = Projection { sol: vec![SolDelta { account: s(2), pre: 1, post: 2 }], ..Default::default() };
        assert!(touches_signers(&other_signer, &meta(), 5000, &BTreeMap::new()));
        let stranger = Projection { sol: vec![SolDelta { account: s(7), pre: 1, post: 2 }], ..Default::default() };
        assert!(!touches_signers(&stranger, &meta(), 5000, &BTreeMap::new()));
    }

    #[test]
    fn token_and_authority_effects_count_when_a_signer_owns_the_account() {
        let tok = |owner: u8| Projection { tokens: vec![TokenDelta { account: s(20), mint: s(30), owner: s(owner), pre: "5".into(), post: "0".into(), decimals: Some(6) }], ..Default::default() };
        assert!(touches_signers(&tok(2), &meta(), 5000, &BTreeMap::new()));
        assert!(!touches_signers(&tok(7), &meta(), 5000, &BTreeMap::new()));
        let approve = Projection { authority: vec![AuthorityChange { account: s(20), field: "delegate".into(), pre: None, post: Some(s(9)) }], ..Default::default() };
        let owned = BTreeMap::from([(s(20), TokenAccount { mint: s(30), owner: s(1) })]);
        assert!(touches_signers(&approve, &meta(), 5000, &owned));
        assert!(!touches_signers(&approve, &meta(), 5000, &BTreeMap::new()));
        let closed = Projection { closed: vec![s(20)], ..Default::default() };
        assert!(touches_signers(&closed, &meta(), 5000, &owned));
    }

    #[test]
    fn record_keeps_successful_signer_effects_only() {
        let (acct, mint) = (key(20), key(30));
        let pre = vec![(key(1), Some(Account { lamports: 1_000_000, ..Account::default() })), (acct, Some(token_account(mint, key(1))))];
        let post = vec![(key(1), Account { lamports: 495_000, ..Account::default() }), (key(3), Account { lamports: 500_000, ..Account::default() }),
                        (acct, Account { lamports: 0, ..Account::default() })];
        let m = TxMeta { programs: vec![s(50)], ..meta() };
        let r = build_record(&report(None, pre.clone(), post.clone()), m.clone(), 777, None).expect("kept");
        assert_eq!((r.slot, r.state_slot, r.units, r.fee), (777, 900, 1234, 5000));
        assert_eq!(r.tx_digest, "ab".repeat(32));
        assert_eq!(r.programs, vec![s(50), s(51)]);
        assert_eq!(r.projection, project(&pre.iter().cloned().collect(), &post.iter().cloned().collect()));
        assert_eq!(r.token_accounts, BTreeMap::from([(acct.to_string(), TokenAccount { mint: mint.to_string(), owner: s(1) })]));
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("tx").is_none());
        for f in ["tx_digest", "slot", "signers", "fee_payer", "programs", "projection", "units"] { assert!(v.get(f).is_some(), "{f}"); }
        let with_tx = build_record(&report(None, pre.clone(), post.clone()), m.clone(), 777, Some("AQID")).unwrap();
        assert_eq!(serde_json::to_value(&with_tx).unwrap()["tx"], "AQID");
        let err = solana_transaction_error::TransactionError::AccountNotFound;
        assert!(build_record(&report(Some(err), pre, post), m.clone(), 777, None).is_none());
        let fee_pre = vec![(key(1), Some(Account { lamports: 10_000, ..Account::default() }))];
        let fee_post = vec![(key(1), Account { lamports: 5_000, ..Account::default() })];
        assert!(build_record(&report(None, fee_pre, fee_post), m, 777, None).is_none());
    }

    #[test]
    fn resume_reads_digests_and_repairs_a_truncated_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e.jsonl");
        assert!(seen_digests(&p).unwrap().is_empty());
        std::fs::write(&p, "{\"tx_digest\":\"aa\"}\n\nnot json\n{\"tx_digest\":\"bb\",\"slot\":1}\n{\"tx_dig").unwrap();
        assert_eq!(seen_digests(&p).unwrap(), HashSet::from(["aa".to_string(), "bb".to_string()]));
        let mut f = open_append(&p).unwrap();
        writeln!(f, "{{\"tx_digest\":\"cc\"}}").unwrap();
        drop(f);
        assert_eq!(seen_digests(&p).unwrap(), HashSet::from(["aa".to_string(), "bb".to_string(), "cc".to_string()]));
        assert!(std::fs::read_to_string(&p).unwrap().ends_with("{\"tx_dig\n{\"tx_digest\":\"cc\"}\n"));
    }

    #[test]
    fn sampling_spreads_over_the_block() {
        assert_eq!(sample_indices(3, 8), vec![0, 1, 2]);
        assert_eq!(sample_indices(10, 4), vec![0, 2, 5, 7]);
        assert!(sample_indices(0, 4).is_empty());
        assert!(sample_indices(10, 0).is_empty());
    }

    #[test]
    fn rate_limits_retry_and_missing_blocks_skip() {
        assert_eq!(block_failure(&SourceError::Unavailable("HTTP 429 Too Many Requests".into())), BlockFailure::Retry);
        assert_eq!(block_failure(&SourceError::Rpc(json!({"code": 429, "message": "Too many requests"}))), BlockFailure::Retry);
        assert_eq!(block_failure(&SourceError::Rpc(json!({"code": -32005, "message": "Node is behind"}))), BlockFailure::Retry);
        for code in [-32007, -32009, -32004, -32015] {
            assert_eq!(block_failure(&SourceError::Rpc(json!({"code": code, "message": "skipped"}))), BlockFailure::Skip);
        }
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(9), Duration::from_secs(16));
    }

    #[test]
    fn only_the_host_is_printed() {
        assert_eq!(host_only("https://mainnet.helius-rpc.com/?api-key=SECRET"), "mainnet.helius-rpc.com");
        assert_eq!(host_only("https://x.quiknode.pro/SECRET/"), "x.quiknode.pro");
        assert_eq!(host_only("not a url"), "<invalid url>");
        assert_eq!(short("error sending request for url (https://h/?api-key=SECRET)"), "error sending request for url <url>");
    }

    #[test]
    fn progress_line_reports_every_counter() {
        let st = Stats { blocks: 2, scanned: 9, kept: 4, failed: 3, no_effect: 1, errored: 1, duplicate: 0, block_errors: 0 };
        assert_eq!(progress(5, &st, 30), "slot 5: kept 4/30 — scanned 9 (failed 3, no signer effect 1, errored 1) — blocks 2 (block errors 0, duplicates skipped 0)");
    }
}
