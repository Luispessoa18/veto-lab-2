//! Conformance against Veto's recorded devnet fixtures. Veto is private: point
//! VETO_FIXTURES at a local checkout's test/fixtures; nothing is copied here.
use aval_svm::{cache::Cache, decode::{decode, Encoding}, engine::Engine, pool::Pool, rpc::simulate_result, source::MemSource};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};
use solana_account::Account;
use solana_address::Address;
use std::str::FromStr;
use std::time::Duration;

#[tokio::test]
async fn matches_veto_fixtures() {
    let Ok(dir) = std::env::var("VETO_FIXTURES") else {
        eprintln!("VETO_FIXTURES not set; skipping");
        return;
    };
    let load = |path: &std::path::Path| -> Value { serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap() };
    let paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json")).collect();
    let account = |a: &Value| Account {
        lamports: a["lamports"].as_str().unwrap().parse().unwrap(),
        data: B64.decode(a["dataBase64"].as_str().unwrap()).unwrap(),
        owner: Address::from_str(a["owner"].as_str().unwrap()).unwrap(),
        executable: a["executable"].as_bool().unwrap(),
        rent_epoch: u64::MAX,
    };
    // A fixture records only the accounts it asked for (before.addresses), so an account the
    // transaction reads but never requested (e.g. the wSOL mint) is missing from its own
    // pre-state. The same devnet account recorded in another fixture fills that gap.
    let mut pool: std::collections::HashMap<String, Value> = Default::default();
    for p in &paths {
        for a in load(p)["before"]["accounts"].as_array().unwrap().iter().filter(|a| !a.is_null()) {
            pool.entry(a["address"].as_str().unwrap().to_string()).or_insert_with(|| a.clone());
        }
    }
    let mut checked = 0;
    for path in paths {
        let f = load(&path);
        if !f["lookupTables"].as_array().is_some_and(|a| a.is_empty()) { continue; }
        let src = MemSource::new(f["before"]["slot"].as_u64().unwrap());
        let recorded: Vec<&str> = f["before"]["addresses"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        for (addr, a) in &pool {
            if !recorded.contains(&addr.as_str()) {
                src.insert(Address::from_str(addr).unwrap(), account(a));
            }
        }
        for a in f["before"]["accounts"].as_array().unwrap().iter().filter(|a| !a.is_null()) {
            src.insert(Address::from_str(a["address"].as_str().unwrap()).unwrap(), account(a));
        }
        // Devnet's Rent sysvar (live getAccountInfo: data 2BMAAAAAAAAAAAAAAADwPzI=, 5080 lamports/byte).
        // The fixtures don't record sysvars, and devnet's rent differs from LiteSVM's default.
        src.insert(solana_sdk_ids::sysvar::rent::id(), Account {
            lamports: 1_009_200,
            data: B64.decode("2BMAAAAAAAAAAAAAAADwPzI=").unwrap(),
            owner: solana_sdk_ids::sysvar::id(),
            executable: false,
            rent_epoch: u64::MAX,
        });
        let engine = Engine::new(Cache::new(src, Duration::from_secs(60)), Pool::new(1, 100));
        let sim = &f["simulation"];
        let tx = f["txBase64"].as_str().unwrap();
        let result = simulate_result(&engine, &json!([tx, {"encoding": "base64", "replaceRecentBlockhash": true,
            "accounts": {"encoding": "base64", "addresses": sim["requestedAccounts"]}}])).await.unwrap();
        let v = &result["value"];
        let name = path.file_name().unwrap().to_string_lossy();
        // Veto stores a failing simulation's error as a JSON-encoded string.
        let want_err = match &sim["error"] {
            Value::String(s) => serde_json::from_str(s).unwrap(),
            other => other.clone(),
        };
        assert_eq!(v["err"], want_err, "{name}: err");
        for (i, want) in sim["accounts"].as_array().unwrap().iter().enumerate() {
            let got = &v["accounts"][i];
            if want.is_null() { assert!(got.is_null(), "{name}: account {i} should be null"); continue; }
            assert_eq!(got["lamports"].as_u64().unwrap().to_string(), want["lamports"].as_str().unwrap(), "{name}: account {i} lamports");
            assert_eq!(got["data"][0].as_str().unwrap(), want["dataBase64"].as_str().unwrap(), "{name}: account {i} data");
        }
        assert_eq!(decode(tx, Encoding::Base64).unwrap().digest.len(), 64);
        checked += 1;
    }
    assert!(checked > 0, "no fixtures found in {dir}");
}
