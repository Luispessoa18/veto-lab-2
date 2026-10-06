use crate::decode::{decode, Encoding};
use crate::engine::{Engine, SimReport};
use crate::source::AccountSource;
use crate::upstream::Upstream;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};
use solana_account::Account;
use solana_address::Address;
use solana_message::inner_instruction::InnerInstructionsList;
use std::str::FromStr;

#[derive(Debug)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError { code: -32602, message: message.into() }
}

pub fn ui_account(a: &Account) -> Value {
    json!({"lamports": a.lamports, "owner": a.owner.to_string(), "data": [B64.encode(&a.data), "base64"],
           "executable": a.executable, "rentEpoch": a.rent_epoch, "space": a.data.len()})
}

fn ui_inner(list: &InnerInstructionsList) -> Value {
    Value::Array(list.iter().enumerate().filter(|(_, v)| !v.is_empty()).map(|(i, v)| json!({
        "index": i,
        "instructions": v.iter().map(|ii| json!({
            "programIdIndex": ii.instruction.program_id_index,
            "accounts": ii.instruction.accounts,
            "data": bs58::encode(&ii.instruction.data).into_string(),
            "stackHeight": ii.stack_height,
        })).collect::<Vec<_>>(),
    })).collect())
}

pub fn aval_meta(r: &SimReport) -> Value {
    json!({"digest": r.digest, "stateSlot": r.slot, "cache": {"hits": r.hits, "misses": r.misses}, "elapsedUs": r.elapsed_us})
}

/// Static keys plus every address loaded from lookup tables.
fn account_count(msg: &solana_message::VersionedMessage) -> usize {
    msg.static_account_keys().len()
        + msg.address_table_lookups().unwrap_or(&[]).iter()
            .map(|l| l.writable_indexes.len() + l.readonly_indexes.len()).sum::<usize>()
}

/// The `result` of simulateTransaction, shaped like Solana RPC plus `aval`.
pub async fn simulate_result<S: AccountSource>(engine: &Engine<S>, params: &Value) -> Result<Value, RpcError> {
    let tx_str = params.get(0).and_then(Value::as_str).ok_or_else(|| invalid("params[0] must be the transaction string"))?;
    let config = params.get(1).cloned().unwrap_or(json!({}));
    let enc = Encoding::parse(config.get("encoding").and_then(Value::as_str)).ok_or_else(|| invalid("unsupported encoding"))?;
    if config.get("sigVerify").and_then(Value::as_bool) == Some(true) {
        return Err(invalid("sigVerify=true is not supported by aval-svm; simulate the unsigned transaction"));
    }
    let requested: Option<Vec<Address>> = match config.get("accounts") {
        None | Some(Value::Null) => None,
        Some(acc) => {
            if acc.get("encoding").and_then(Value::as_str).is_some_and(|e| e != "base64") {
                return Err(invalid("accounts.encoding must be base64"));
            }
            let list = acc.get("addresses").and_then(Value::as_array).ok_or_else(|| invalid("accounts.addresses"))?;
            Some(list.iter().map(|a| a.as_str().and_then(|s| Address::from_str(s).ok()).ok_or_else(|| invalid("bad address")))
                .collect::<Result<_, _>>()?)
        }
    };
    let fresh = config.pointer("/aval/fresh").and_then(Value::as_bool).unwrap_or(false);
    let decoded = decode(tx_str, enc).map_err(|e| invalid(e.to_string()))?;
    if requested.as_ref().is_some_and(|r| r.len() > account_count(&decoded.tx.message)) {
        return Err(invalid("Too many accounts provided"));
    }
    let report = engine.simulate(decoded, fresh).await.map_err(|e| RpcError { code: e.code(), message: e.to_string() })?;
    let o = &report.outcome;
    let mut extra: std::collections::HashMap<Address, Option<Account>> = Default::default();
    if o.err.is_none() {
        if let Some(keys) = &requested {
            let missing: Vec<Address> = keys.iter().filter(|k| !o.post.contains_key(*k) && !report.pre.contains_key(*k)).cloned().collect();
            if !missing.is_empty() {
                let got = engine.cache().get_many(&missing, fresh).await.map_err(|e| RpcError { code: -32005, message: e.to_string() })?;
                extra = got.accounts;
            }
        }
    }
    let accounts = requested.map(|keys| {
        Value::Array(keys.iter().map(|k| {
            if o.err.is_some() { return Value::Null; }
            match o.post.get(k).or_else(|| report.pre.get(k).and_then(|a| a.as_ref())).or_else(|| extra.get(k).and_then(|a| a.as_ref())) {
                Some(a) => ui_account(a),
                // An account the transaction loads but that does not exist upstream is returned
                // as an empty system account (as the cluster does); one it never loads is null.
                None if report.pre.contains_key(k) => ui_account(&Account::default()),
                None => Value::Null,
            }
        }).collect())
    });
    let return_data = if o.return_data.data.is_empty() { Value::Null } else {
        json!({"programId": o.return_data.program_id.to_string(), "data": [B64.encode(&o.return_data.data), "base64"]})
    };
    let mut value = json!({
        "err": o.err.as_ref().map(|e| serde_json::to_value(e).unwrap_or(json!(e.to_string()))),
        "logs": o.logs,
        "accounts": accounts,
        "unitsConsumed": o.units,
        "returnData": return_data,
        "fee": o.fee,
    });
    if config.get("innerInstructions").and_then(Value::as_bool) == Some(true) {
        value["innerInstructions"] = ui_inner(&o.inner);
    }
    if config.get("replaceRecentBlockhash").and_then(Value::as_bool) == Some(true) {
        value["replacementBlockhash"] = json!({"blockhash": o.blockhash.to_string(), "lastValidBlockHeight": report.slot + 150});
    }
    Ok(json!({"context": {"slot": report.slot}, "value": value, "aval": aval_meta(&report)}))
}

async fn handle_one<S: AccountSource>(engine: &Engine<S>, upstream: &Upstream, req: Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let error = |code: i64, message: String| json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}});
    match req.get("method").and_then(Value::as_str) {
        Some("simulateTransaction") => match simulate_result(engine, req.get("params").unwrap_or(&Value::Null)).await {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => error(e.code, e.message),
        },
        Some(_) => match upstream.forward(&req).await {
            Ok(reply) => reply,
            Err(e) => error(-32005, e.to_string()),
        },
        None => error(-32600, "invalid request".into()),
    }
}

pub async fn handle<S: AccountSource>(engine: &Engine<S>, upstream: &Upstream, body: Value) -> Value {
    match body {
        Value::Array(reqs) => {
            let mut out = Vec::with_capacity(reqs.len());
            for r in reqs {
                out.push(handle_one(engine, upstream, r).await);
            }
            Value::Array(out)
        }
        single => handle_one(engine, upstream, single).await,
    }
}
