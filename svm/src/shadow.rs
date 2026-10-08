use serde_json::Value;
use crate::cache::Cache;
use crate::engine::Engine;
use crate::pool::Pool;
use crate::rpc::{simulate_result, RpcError};
use crate::source::SourceError;
use crate::upstream::Upstream;
use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

pub const VOTE_PROGRAM: &str = "Vote111111111111111111111111111111111111111";

/// Replaces the numbers in `Program <id> consumed <X> of <Y> compute units` so that
/// compute-unit drift between program versions is not a disagreement; other lines stay exact.
fn normalise_log(line: &str) -> String {
    let parts: Vec<&str> = line.split(' ').collect();
    if parts.len() == 8 && parts[0] == "Program" && parts[2] == "consumed" && parts[4] == "of"
        && parts[6] == "compute" && parts[7] == "units"
        && parts[3].bytes().all(|b| b.is_ascii_digit()) && parts[5].bytes().all(|b| b.is_ascii_digit())
    {
        return format!("Program {} consumed _ of _ compute units", parts[1]);
    }
    line.to_string()
}

fn normalised_logs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|l| normalise_log(l.as_str().unwrap_or_default())).collect()).unwrap_or_default()
}

/// Outcome of comparing one transaction. Infrastructure failures (upstream down, rate limited)
/// say nothing about agreement, so they are `Errored` and left out of the denominator.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Both sides executed successfully and the results match.
    AgreeExecuted,
    /// Both sides failed with the same error (and the same logs).
    AgreeSameFailure,
    Disagree(Vec<String>),
    Errored(Vec<String>),
}

pub fn classify(ours: &Result<Value, RpcError>, theirs: &Result<Value, SourceError>) -> Verdict {
    let mut errs = Vec::new();
    if let Err(e) = theirs { errs.push(format!("upstream error: {e}")); }
    if let Err(e) = ours {
        if e.code == -32005 {
            errs.push(format!("aval error {}: {}", e.code, e.message));
        } else if theirs.is_ok() {
            return Verdict::Disagree(vec![format!("aval error {}: {}", e.code, e.message)]);
        } else {
            errs.push(format!("aval error {}: {}", e.code, e.message));
        }
    }
    if !errs.is_empty() { return Verdict::Errored(errs); }
    match (ours, theirs) {
        (Ok(o), Ok(t)) => {
            let f = compare(&o["value"], &t["value"]);
            match (f.is_empty(), o["value"]["err"].is_null()) {
                (true, true) => Verdict::AgreeExecuted,
                (true, false) => Verdict::AgreeSameFailure,
                (false, _) => Verdict::Disagree(f),
            }
        }
        _ => unreachable!(),
    }
}

/// Units consumed are not compared: program versions can shift them by a few CUs.
pub fn compare(ours: &Value, theirs: &Value) -> Vec<String> {
    let mut diff = Vec::new();
    if ours["err"] != theirs["err"] { diff.push("err".into()); }
    if normalised_logs(&ours["logs"]) != normalised_logs(&theirs["logs"]) { diff.push("logs".into()); }
    let empty = vec![];
    let (a, b) = (ours["accounts"].as_array().unwrap_or(&empty), theirs["accounts"].as_array().unwrap_or(&empty));
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        if x["lamports"] != y["lamports"] { diff.push(format!("accounts[{i}].lamports")); }
        if x["data"] != y["data"] { diff.push(format!("accounts[{i}].data")); }
    }
    diff
}

/// Context slot of a simulateTransaction result, when present.
fn context_slot(v: &Value) -> Option<u64> { v["context"]["slot"].as_u64() }

/// True when every aval read was at upstream's slot (newest and oldest), so a disagreement cannot be slot skew.
pub fn same_slot(upstream: Option<u64>, aval: Option<u64>, aval_min: Option<u64>) -> bool {
    matches!((upstream, aval, aval_min), (Some(a), Some(b), Some(c)) if a == b && a == c)
}

/// One line that keeps "agreed because both failed the same way" apart from real executions.
pub fn summary(executed: usize, same_failure: usize, compared: usize, errored: usize) -> String {
    let agree = executed + same_failure;
    format!("agree {agree}/{compared} ({:.1}%) — {executed} executed+matched, {same_failure} same-failure — {errored} errored",
        100.0 * agree as f64 / compared.max(1) as f64)
}

fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() { return 0; }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// Replays real transactions from one block through upstream and aval-svm against the same
/// current state, and reports agreement and latency (cold = first run, warm = second run).
pub async fn run(upstream_url: &str, count: usize, slot: Option<u64>, out: &Path, delay_ms: u64) -> anyhow::Result<()> {
    let up = Upstream::new(upstream_url, "confirmed", 30_000);
    let engine = Engine::new(Cache::new(up.clone(), Duration::from_millis(2000)), Pool::new(4, 5000));
    let slot = match slot {
        Some(s) => s,
        None => up.call("getSlot", json!([{"commitment": "finalized"}])).await?.as_u64().unwrap_or(0) - 20,
    };
    let block = up.call("getBlock", json!([slot, {"encoding": "base64", "transactionDetails": "full",
        "maxSupportedTransactionVersion": 1, "rewards": false}])).await?;
    let txs: Vec<String> = block["transactions"].as_array().cloned().unwrap_or_default().into_iter()
        .filter_map(|t| t["transaction"][0].as_str().map(String::from))
        .filter(|b64| {
            crate::decode::decode(b64, crate::decode::Encoding::Base64).is_ok_and(|d| {
                !d.tx.message.static_account_keys().iter().any(|k| k.to_string() == VOTE_PROGRAM)
            })
        })
        .take(count)
        .collect();
    let mut file = std::fs::File::create(out)?;
    let (mut same_slot_dis, mut diff_slot_dis) = (0usize, 0usize);
    let (mut executed, mut same_failure, mut errored, mut cold, mut warm, mut rpc) = (0usize, 0usize, 0usize, vec![], vec![], vec![]);
    for (n, b64) in txs.iter().enumerate() {
        if n > 0 { tokio::time::sleep(Duration::from_millis(delay_ms)).await; }
        let d = crate::decode::decode(b64, crate::decode::Encoding::Base64)?;
        let writable: Vec<String> = d.tx.message.static_account_keys().iter()
            .enumerate().filter(|(i, _)| d.tx.message.is_maybe_writable_with_reserved_addresses(*i, None::<&std::collections::HashSet<solana_address::Address>>)).map(|(_, k)| k.to_string()).take(20).collect();
        let cfg = json!({"encoding": "base64", "commitment": "confirmed", "sigVerify": false, "replaceRecentBlockhash": true,
            "accounts": {"encoding": "base64", "addresses": writable}});
        let mut fresh_cfg = cfg.clone();
        fresh_cfg["aval"] = json!({"fresh": true});
        // Both sides run at the same moment so they see (almost) the same chain state.
        let up_params = json!([b64, cfg]);
        let fresh_params = json!([b64, fresh_cfg]);
        let t = Instant::now();
        let (theirs, ours) = tokio::join!(
            async { let r = up.call("simulateTransaction", up_params).await; (r, t.elapsed().as_micros() as u64) },
            simulate_result(&engine, &fresh_params),
        );
        let (theirs, rpc_us) = theirs;
        rpc.push(rpc_us);
        let ours_warm = simulate_result(&engine, &json!([b64, cfg])).await;
        if let (Ok(o), Ok(_)) = (&ours, &theirs) {
            cold.push(o["aval"]["elapsedUs"].as_u64().unwrap_or(0));
            if let Ok(w) = &ours_warm { warm.push(w["aval"]["elapsedUs"].as_u64().unwrap_or(0)); }
        }
        let (label, fields) = match classify(&ours, &theirs) {
            Verdict::AgreeExecuted => { executed += 1; ("agree", vec![]) }
            Verdict::AgreeSameFailure => { same_failure += 1; ("agree-same-failure", vec![]) }
            Verdict::Disagree(f) => ("disagree", f),
            Verdict::Errored(f) => { errored += 1; ("errored", f) }
        };
        let up_slot = theirs.as_ref().ok().and_then(context_slot);
        let av_slot = ours.as_ref().ok().and_then(context_slot);
        let av_min = ours.as_ref().ok().and_then(|o| o["aval"]["stateSlotMin"].as_u64());
        if label == "disagree" {
            if same_slot(up_slot, av_slot, av_min) { same_slot_dis += 1 } else { diff_slot_dis += 1 }
        }
        writeln!(file, "{}", json!({"tx": &b64[..b64.len().min(24)], "result": label, "agree": label.starts_with("agree"), "diff": fields,
            "upstreamSlot": up_slot, "avalSlot": av_slot, "avalSlotMin": av_min}))?;
    }
    for v in [&mut cold, &mut warm, &mut rpc] { v.sort(); }
    println!("slot {slot}: {}", summary(executed, same_failure, txs.len() - errored, errored));
    println!("disagreements: {} at the same slot (all reads), {} at different slots", same_slot_dis, diff_slot_dis);
    println!("aval cold  p50 {} µs  p95 {} µs", pct(&cold, 0.5), pct(&cold, 0.95));
    println!("aval warm  p50 {} µs  p95 {} µs", pct(&warm, 0.5), pct(&warm, 0.95));
    println!("rpc        p50 {} µs  p95 {} µs", pct(&rpc, 0.5), pct(&rpc, 0.95));
    println!("details: {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn same_slot_needs_both_equal() {
        assert!(same_slot(Some(5), Some(5), Some(5)));
        assert!(!same_slot(Some(5), Some(6), Some(6)));
        assert!(!same_slot(Some(5), Some(5), Some(4)), "an older read breaks it");
        assert!(!same_slot(None, Some(5), Some(5)));
        assert!(!same_slot(Some(5), Some(5), None));
        assert!(!same_slot(None, None, None));
        assert_eq!(context_slot(&json!({"context": {"slot": 7}})), Some(7));
    }

    #[test]
    fn identical_results_agree() {
        let v = json!({"err": null, "logs": ["a"], "accounts": [{"lamports": 1, "data": ["AA==", "base64"]}], "unitsConsumed": 150});
        assert!(compare(&v, &v).is_empty());
    }

    #[test]
    fn units_may_differ_but_err_and_balances_may_not() {
        let a = json!({"err": null, "logs": ["a"], "accounts": [{"lamports": 1, "data": ["AA==", "base64"]}], "unitsConsumed": 150});
        let mut b = a.clone();
        b["unitsConsumed"] = json!(151);
        assert!(compare(&a, &b).is_empty());
        b["accounts"][0]["lamports"] = json!(2);
        b["err"] = json!({"InstructionError": [0, {"Custom": 1}]});
        assert_eq!(compare(&a, &b), vec!["err".to_string(), "accounts[0].lamports".to_string()]);
    }

    #[test]
    fn compute_unit_numbers_in_logs_are_normalised_but_other_lines_are_exact() {
        let mk = |x: u32, line: &str| json!({"err": null, "accounts": [], "logs": [
            format!("Program 11111111111111111111111111111111 consumed {x} of 200000 compute units"), line]});
        assert!(compare(&mk(150, "Program log: hi"), &mk(151, "Program log: hi")).is_empty());
        assert_eq!(compare(&mk(150, "Program log: hi"), &mk(150, "Program log: ho")), vec!["logs".to_string()]);
    }

    #[test]
    fn agreements_split_into_executed_and_same_failure() {
        let ok = json!({"value": {"err": null, "logs": ["a"], "accounts": []}});
        let failed = json!({"value": {"err": {"InstructionError": [0, {"Custom": 1}]}, "logs": ["a"], "accounts": []}});
        let other_failure = json!({"value": {"err": {"InstructionError": [0, {"Custom": 2}]}, "logs": ["a"], "accounts": []}});
        assert_eq!(classify(&Ok(ok.clone()), &Ok(ok.clone())), Verdict::AgreeExecuted);
        assert_eq!(classify(&Ok(failed.clone()), &Ok(failed.clone())), Verdict::AgreeSameFailure);
        assert!(matches!(classify(&Ok(failed.clone()), &Ok(other_failure)), Verdict::Disagree(_)));
        assert!(matches!(classify(&Ok(ok), &Ok(failed)), Verdict::Disagree(_)));
        assert_eq!(summary(12, 6, 20, 0), "agree 18/20 (90.0%) — 12 executed+matched, 6 same-failure — 0 errored");
        assert_eq!(summary(0, 0, 0, 3), "agree 0/0 (0.0%) — 0 executed+matched, 0 same-failure — 3 errored");
    }

    #[test]
    fn infrastructure_failures_are_errored_not_disagreement() {
        let ok = json!({"value": {"err": null, "logs": [], "accounts": []}});
        let bad = json!({"value": {"err": "X", "logs": [], "accounts": []}});
        let down = || Err::<Value, _>(RpcError { code: -32005, message: "down".into() });
        let up_err = || Err::<Value, _>(SourceError::Unavailable("429".into()));
        assert_eq!(classify(&Ok(ok.clone()), &Ok(ok.clone())), Verdict::AgreeExecuted);
        assert!(matches!(classify(&Ok(ok.clone()), &Ok(bad)), Verdict::Disagree(_)));
        assert!(matches!(classify(&Ok(ok.clone()), &up_err()), Verdict::Errored(e) if e.len() == 1));
        assert!(matches!(classify(&down(), &Ok(ok.clone())), Verdict::Errored(_)));
        assert!(matches!(classify(&down(), &up_err()), Verdict::Errored(e) if e.len() == 2));
        let other = Err::<Value, _>(RpcError { code: -32602, message: "bad".into() });
        assert!(matches!(classify(&other, &Ok(ok)), Verdict::Disagree(_)));
    }
}
