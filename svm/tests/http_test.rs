mod common;
use aval_svm::{cache::Cache, engine::Engine, http::{router, App}, pool::Pool, source::MemSource, upstream::Upstream};
use axum::body::Body;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use common::{key, padded_tx, transfer_tx};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use solana_account::Account;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

async fn app(upstream_uri: &str) -> axum::Router {
    let src = MemSource::new(900);
    src.insert(key(1), Account { lamports: 10_000_000_000, owner: solana_sdk_ids::system_program::id(), ..Account::default() });
    src.insert(key(5), Account { lamports: 777, owner: solana_sdk_ids::system_program::id(), ..Account::default() });
    let engine = Engine::new(Cache::new(src, Duration::from_secs(60)), Pool::new(1, 100));
    router(Arc::new(App { engine, upstream: Upstream::new(upstream_uri, "confirmed", 2000) }))
}

async fn post(app: axum::Router, path: &str, body: Value) -> Value {
    let req = axum::http::Request::post(path).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn simulate_matches_rpc_shape_with_requested_accounts() {
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let body = json!({"jsonrpc": "2.0", "id": 7, "method": "simulateTransaction", "params": [B64.encode(&raw), {
        "encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": true, "innerInstructions": true,
        "accounts": {"encoding": "base64", "addresses": [key(2).to_string(), key(6).to_string(), key(5).to_string()]}}]});
    let out = post(app("http://127.0.0.1:9").await, "/", body).await;
    assert_eq!(out["id"], 7);
    let v = &out["result"]["value"];
    assert!(v["err"].is_null(), "{out}");
    assert_eq!(out["result"]["context"]["slot"], 900);
    assert_eq!(v["accounts"][0]["lamports"], 1_000_000);
    assert_eq!(v["accounts"][0]["data"][1], "base64");
    assert!(v["accounts"][1].is_null(), "truly absent key");
    assert_eq!(v["accounts"][2]["lamports"], 777, "untouched account returns current state");
    assert!(v["unitsConsumed"].as_u64().unwrap() > 0);
    assert!(v["replacementBlockhash"]["blockhash"].is_string());
    assert_eq!(out["result"]["aval"]["digest"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn loaded_but_absent_account_is_an_empty_system_account() {
    // key(7) is a READONLY account of the transaction that does not exist upstream, so it is
    // not in the VM's post-state and takes the "loaded but absent" path.
    use solana_hash::Hash;
    use solana_message::{Message, VersionedMessage};
    use solana_signature::Signature;
    use solana_transaction::versioned::VersionedTransaction;
    let mut ix = solana_system_interface::instruction::transfer(&key(1), &key(2), 1_000_000);
    ix.accounts.push(solana_instruction::AccountMeta::new_readonly(key(7), false));
    let msg = Message::new_with_blockhash(&[ix], Some(&key(1)), &Hash::new_from_array([7; 32]));
    let idx = msg.account_keys.iter().position(|k| *k == key(7)).unwrap();
    let tx = VersionedTransaction { signatures: vec![Signature::default()], message: VersionedMessage::Legacy(msg) };
    assert!(!tx.message.is_maybe_writable_with_reserved_addresses(idx, None::<&std::collections::HashSet<solana_address::Address>>), "key(7) must be readonly");
    let raw = bincode::serialize(&tx).unwrap();
    let out = post(app("http://127.0.0.1:9").await, "/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction",
        "params": [B64.encode(&raw), {"encoding": "base64", "accounts": {"encoding": "base64",
            "addresses": [key(7).to_string(), key(6).to_string()]}}]})).await;
    let v = &out["result"]["value"];
    assert!(v["err"].is_null(), "{out}");
    assert_eq!(v["accounts"][0]["lamports"], 0);
    assert_eq!(v["accounts"][0]["owner"], "11111111111111111111111111111111");
    assert_eq!(v["accounts"][0]["data"][0], "");
    assert!(v["accounts"][1].is_null(), "an address the tx never loads stays null");
}

#[tokio::test]
async fn default_encoding_is_base58_and_config_is_optional() {
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let out = post(app("http://127.0.0.1:9").await, "/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction", "params": [bs58::encode(&raw).into_string()]})).await;
    assert!(out["result"]["value"]["err"].is_null(), "{out}");
}

#[tokio::test]
async fn failed_tx_returns_null_accounts_and_err() {
    let (_, raw) = transfer_tx(key(1), key(2), 99_000_000_000_000);
    let out = post(app("http://127.0.0.1:9").await, "/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction",
        "params": [B64.encode(&raw), {"encoding": "base64", "accounts": {"encoding": "base64", "addresses": [key(1).to_string()]}}]})).await;
    let v = &out["result"]["value"];
    assert!(!v["err"].is_null());
    assert_eq!(v["accounts"], json!([null]));
}

#[tokio::test]
async fn bad_params_and_sigverify_true_are_invalid_params() {
    let a = app("http://127.0.0.1:9").await;
    let out = post(a.clone(), "/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction", "params": ["@@", {"encoding": "base64"}]})).await;
    assert_eq!(out["error"]["code"], -32602);
    let (_, raw) = transfer_tx(key(1), key(2), 1);
    let out = post(a, "/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction", "params": [B64.encode(&raw), {"encoding": "base64", "sigVerify": true}]})).await;
    assert_eq!(out["error"]["code"], -32602);
}

#[tokio::test]
async fn other_methods_are_proxied_and_batches_work() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": 3, "result": {"value": 5000}}))).mount(&server).await;
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let out = post(app(&server.uri()).await, "/", json!([
        {"jsonrpc": "2.0", "id": 3, "method": "getFeeForMessage", "params": ["x"]},
        {"jsonrpc": "2.0", "id": 4, "method": "simulateTransaction", "params": [B64.encode(&raw), {"encoding": "base64"}]}
    ])).await;
    assert_eq!(out[0]["result"]["value"], 5000);
    assert_eq!(out[1]["id"], 4);
    assert!(out[1]["result"]["value"]["err"].is_null());
}

#[tokio::test]
async fn proxy_with_upstream_down_is_32005() {
    let out = post(app("http://127.0.0.1:9").await, "/", json!({"jsonrpc": "2.0", "id": 1, "method": "getLatestBlockhash"})).await;
    assert_eq!(out["error"]["code"], -32005);
}

#[tokio::test]
async fn project_endpoint_returns_projection() {
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let out = post(app("http://127.0.0.1:9").await, "/v1/project", json!({"transaction": B64.encode(&raw)})).await;
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["projection"]["created"], json!([key(2).to_string()]));
    assert!(out["aval"]["elapsedUs"].is_u64());
}

#[tokio::test]
async fn oversized_transactions_are_rejected() {
    let a = app("http://127.0.0.1:9").await;
    let ok = padded_tx(key(1), 3000);
    assert!(ok.len() <= 4096);
    let out = post(a.clone(), "/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction",
        "params": [B64.encode(&ok), {"encoding": "base64"}]})).await;
    assert!(out["result"]["value"]["err"].is_null(), "{}", out["result"]["value"]["err"]);
    let big = padded_tx(key(1), 5000);
    assert!(big.len() > 4096);
    for (path, body) in [
        ("/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction", "params": [B64.encode(&big), {"encoding": "base64"}]})),
        ("/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction", "params": [bs58::encode(&big).into_string()]})),
    ] {
        let out = post(a.clone(), path, body).await;
        assert_eq!(out["error"]["code"], -32602, "{}", out["error"]);
        assert!(out["error"]["message"].as_str().unwrap().contains("too large"), "{}", out["error"]);
    }
    let out = post(a, "/v1/project", json!({"transaction": B64.encode(&big)})).await;
    assert_eq!(out["ok"], false);
    assert_eq!(out["error"]["code"], -32602, "{}", out["error"]);
}

#[tokio::test]
async fn more_requested_accounts_than_the_message_has_is_invalid() {
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000); // 3 account keys
    let addrs = |n: u8| (10..10 + n).map(|i| key(i).to_string()).collect::<Vec<_>>();
    let a = app("http://127.0.0.1:9").await;
    let out = post(a.clone(), "/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction",
        "params": [B64.encode(&raw), {"encoding": "base64", "accounts": {"encoding": "base64", "addresses": addrs(3)}}]})).await;
    assert!(out["result"]["value"]["err"].is_null(), "{out}");
    let out = post(a, "/", json!({"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction",
        "params": [B64.encode(&raw), {"encoding": "base64", "accounts": {"encoding": "base64", "addresses": addrs(4)}}]})).await;
    assert_eq!(out["error"]["code"], -32602);
    assert_eq!(out["error"]["message"], "Too many accounts provided");
}

fn gma(keys: Vec<String>, config: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 9, "method": "getMultipleAccounts", "params": [keys, config]})
}

#[tokio::test]
async fn get_multiple_accounts_base64_is_served_from_the_cache() {
    // Upstream is unreachable: the answer can only come from the cache.
    let out = post(app("http://127.0.0.1:9").await, "/", gma(vec![key(5).to_string(), key(6).to_string(), key(1).to_string()],
        json!({"encoding": "base64", "commitment": "confirmed"}))).await;
    assert_eq!(out["id"], 9);
    assert_eq!(out["result"]["context"], json!({"slot": 900}), "{out}");
    let v = &out["result"]["value"];
    assert_eq!(v[0], json!({"lamports": 777, "owner": "11111111111111111111111111111111", "data": ["", "base64"],
        "executable": false, "rentEpoch": 0, "space": 0}));
    assert!(v[1].is_null(), "absent account is null");
    assert_eq!(v[2]["lamports"], 10_000_000_000u64);
}

#[tokio::test]
async fn get_multiple_accounts_over_100_keys_is_invalid() {
    let keys: Vec<String> = (0..101u8).map(|i| key(i).to_string()).collect();
    let out = post(app("http://127.0.0.1:9").await, "/", gma(keys, json!({"encoding": "base64"}))).await;
    assert_eq!(out["error"]["code"], -32602, "{out}");
}

#[tokio::test]
async fn get_multiple_accounts_cache_failure_is_32005() {
    // The proxy target answers fine; only the cache's source fails.
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
        .set_body_json(json!({"jsonrpc": "2.0", "id": 9, "result": {"context": {"slot": 1}, "value": [null]}}))).mount(&server).await;
    let src = MemSource::new(900);
    src.set_fail(true);
    let engine = Engine::new(Cache::new(src, Duration::from_secs(60)), Pool::new(1, 100));
    let a = router(Arc::new(App { engine, upstream: Upstream::new(&server.uri(), "confirmed", 2000) }));
    let out = post(a, "/", gma(vec![key(5).to_string()], json!({"encoding": "base64"}))).await;
    assert_eq!(out["error"]["code"], -32005, "{out}");
}

#[tokio::test]
async fn get_multiple_accounts_other_shapes_are_proxied() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
        .set_body_json(json!({"jsonrpc": "2.0", "id": 9, "result": {"context": {"slot": 1}, "value": ["upstream"]}}))).mount(&server).await;
    let a = app(&server.uri()).await;
    let k = vec![key(5).to_string()];
    for config in [Value::Null, json!({}), json!({"encoding": "base58"}), json!({"encoding": "jsonParsed"}),
                   json!({"encoding": "base64", "dataSlice": {"offset": 0, "length": 1}}),
                   json!({"encoding": "base64", "minContextSlot": 5})] {
        let body = if config.is_null() { json!({"jsonrpc": "2.0", "id": 9, "method": "getMultipleAccounts", "params": [k]}) } else { gma(k.clone(), config.clone()) };
        let out = post(a.clone(), "/", body).await;
        assert_eq!(out["result"]["value"][0], "upstream", "{config} must be proxied: {out}");
    }
}
