//! Chain abstraction: read an account, send signed instructions and wait for confirmation.
//! `LiteSvmChain` backs tests; `RpcChain` talks JSON-RPC to a real cluster.
use crate::source::SourceError;
use crate::upstream::Upstream;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use litesvm::LiteSVM;
use serde_json::{json, Value};
use solana_address::Address;
use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::future::Future;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, thiserror::Error)]
pub enum ChainError {
    /// The cluster (or VM) refused the transaction; `code` is the program's custom error, when there is one.
    #[error("transaction rejected{}: {message}", code.map(|c| format!(" (custom error {c})")).unwrap_or_default())]
    Rejected { code: Option<u32>, message: String },
    #[error("chain unavailable: {0}")]
    Unavailable(String),
}

pub trait Chain: Send + Sync {
    /// The account's data, or `None` when it does not exist.
    fn account(&self, key: &Address) -> impl Future<Output = Result<Option<Vec<u8>>, ChainError>> + Send;
    /// Signs and sends `ixs` with `signer` as fee payer; returns the base58 signature once confirmed.
    fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> impl Future<Output = Result<String, ChainError>> + Send;
}

/// Reads `err.InstructionError[1].Custom` when present.
fn rejected_from(err: &Value, msg: &Value) -> ChainError {
    let code = err["InstructionError"][1]["Custom"].as_u64().and_then(|c| u32::try_from(c).ok());
    let message = if err.is_null() {
        msg.as_str().map(str::to_string).unwrap_or_else(|| msg.to_string())
    } else if code.is_some() {
        msg.as_str().map(str::to_string).unwrap_or_else(|| err.to_string())
    } else {
        err.to_string()
    };
    ChainError::Rejected { code, message }
}

/// Classifies an error from the cluster: only a definitive TransactionError is a rejection.
/// A missing `err` (node behind, rate limit) or `BlockhashNotFound` is transient, so `Unavailable`.
fn classify(err: &Value, msg: &Value) -> ChainError {
    if err.is_null() || err.as_str() == Some("BlockhashNotFound") {
        let m = msg.as_str().map(str::to_string).unwrap_or_else(|| err.to_string());
        return ChainError::Unavailable(if err.is_null() { m } else { "blockhash not found".into() });
    }
    rejected_from(err, msg)
}

fn unavail(e: SourceError) -> ChainError {
    ChainError::Unavailable(e.to_string())
}

// ---------------------------------------------------------------- LiteSVM

pub struct LiteSvmChain {
    svm: Mutex<LiteSVM>,
}

impl LiteSvmChain {
    pub fn new(svm: LiteSVM) -> Self {
        LiteSvmChain { svm: Mutex::new(svm) }
    }

    /// Direct access to the VM for test setup and assertions.
    pub fn with_vm<R>(&self, f: impl FnOnce(&mut LiteSVM) -> R) -> R {
        f(&mut self.svm.lock().expect("svm lock"))
    }
}

impl Chain for LiteSvmChain {
    fn account(&self, key: &Address) -> impl Future<Output = Result<Option<Vec<u8>>, ChainError>> + Send {
        let r = self.with_vm(|vm| vm.get_account(key).map(|a| a.data));
        async move { Ok(r) }
    }

    fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> impl Future<Output = Result<String, ChainError>> + Send {
        let r = self.with_vm(|vm| {
            let tx = Transaction::new_signed_with_payer(&ixs, Some(&signer.pubkey()), &[signer], vm.latest_blockhash());
            match vm.send_transaction(tx) {
                Ok(meta) => {
                    // Fresh blockhash so identical resends are not rejected as duplicates.
                    vm.expire_blockhash();
                    Ok(meta.signature.to_string())
                }
                Err(f) => {
                    let err = serde_json::to_value(&f.err).unwrap_or(Value::Null);
                    Err(rejected_from(&err, &json!(format!("{:?}", f.err))))
                }
            }
        });
        async move { r }
    }
}

// ---------------------------------------------------------------- RPC

pub struct RpcChain {
    up: Upstream,
    poll: Duration,
    max_wait: Duration,
}

impl RpcChain {
    pub fn new(upstream: Upstream) -> Self {
        Self::with_timing(upstream, Duration::from_millis(500), Duration::from_secs(60))
    }

    pub fn with_timing(upstream: Upstream, poll: Duration, max_wait: Duration) -> Self {
        RpcChain { up: upstream, poll, max_wait }
    }
}

impl Chain for RpcChain {
    async fn account(&self, key: &Address) -> Result<Option<Vec<u8>>, ChainError> {
        let v = self
            .up
            .call("getAccountInfo", json!([key.to_string(), {"encoding": "base64", "commitment": "confirmed"}]))
            .await
            .map_err(unavail)?;
        let value = &v["value"];
        if value.is_null() {
            return Ok(None);
        }
        let data = value["data"][0].as_str().ok_or_else(|| ChainError::Unavailable("malformed account data".into()))?;
        B64.decode(data).map(Some).map_err(|_| ChainError::Unavailable("bad base64 in account data".into()))
    }

    async fn send(&self, ixs: Vec<Instruction>, signer: &Keypair) -> Result<String, ChainError> {
        let unavail_msg = |m: &str| ChainError::Unavailable(m.to_string());
        let bh = self.up.call("getLatestBlockhash", json!([{"commitment": "confirmed"}])).await.map_err(unavail)?;
        let blockhash = Hash::from_str(bh["value"]["blockhash"].as_str().ok_or_else(|| unavail_msg("no blockhash"))?)
            .map_err(|_| unavail_msg("bad blockhash"))?;
        let tx = Transaction::new_signed_with_payer(&ixs, Some(&signer.pubkey()), &[signer], blockhash);
        let raw = B64.encode(bincode::serialize(&tx).map_err(|e| unavail_msg(&e.to_string()))?);
        let sig = match self.up.call("sendTransaction", json!([raw, {"encoding": "base64", "preflightCommitment": "confirmed"}])).await {
            Ok(v) => v.as_str().unwrap_or_default().to_string(),
            // Preflight failures come back as a JSON-RPC error whose data.err is the TransactionError.
            Err(SourceError::Rpc(e)) => return Err(classify(&e["data"]["err"], &e["message"])),
            Err(e) => return Err(ChainError::Unavailable(e.to_string())),
        };
        if sig.is_empty() {
            return Err(unavail_msg("sendTransaction returned no signature"));
        }
        let deadline = tokio::time::Instant::now() + self.max_wait;
        loop {
            tokio::time::sleep(self.poll).await;
            let st = self.up.call("getSignatureStatuses", json!([[sig]])).await.map_err(unavail)?;
            let s = &st["value"][0];
            if !s.is_null() {
                if !s["err"].is_null() {
                    return Err(classify(&s["err"], &json!("transaction failed")));
                }
                if matches!(s["confirmationStatus"].as_str(), Some("confirmed" | "finalized")) {
                    return Ok(sig);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ChainError::Unavailable(format!("not confirmed within {} s", self.max_wait.as_secs())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_system_interface::instruction::transfer;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn funded() -> (LiteSvmChain, Keypair) {
        let mut vm = LiteSVM::new();
        let kp = Keypair::new();
        vm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap();
        (LiteSvmChain::new(vm), kp)
    }

    #[tokio::test]
    async fn lite_account_missing_is_none_and_present_has_data() {
        let (chain, kp) = funded();
        assert_eq!(chain.account(&Address::from([9; 32])).await.unwrap(), None);
        let data = chain.account(&kp.pubkey()).await.unwrap();
        assert_eq!(data, Some(vec![]));
    }

    #[tokio::test]
    async fn lite_send_transfer_moves_lamports() {
        let (chain, kp) = funded();
        let to = Address::from([7; 32]);
        let sig = chain.send(vec![transfer(&kp.pubkey(), &to, 1_000_000)], &kp).await.unwrap();
        assert!(!sig.is_empty());
        assert_eq!(chain.with_vm(|vm| vm.get_balance(&to)), Some(1_000_000));
    }

    #[tokio::test]
    async fn lite_over_balance_transfer_is_rejected() {
        let (chain, kp) = funded();
        let to = Address::from([7; 32]);
        let r = chain.send(vec![transfer(&kp.pubkey(), &to, 20_000_000_000)], &kp).await;
        assert!(matches!(r, Err(ChainError::Rejected { .. })), "{r:?}");
    }

    fn rpc(server: &MockServer) -> RpcChain {
        RpcChain::with_timing(Upstream::new(&server.uri(), "confirmed", 2000), Duration::from_millis(10), Duration::from_millis(300))
    }

    async fn mock(server: &MockServer, m: &str, result: Value) {
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": m})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": result})))
            .mount(server)
            .await;
    }

    async fn mock_send_prereqs(server: &MockServer) {
        mock(server, "getLatestBlockhash", json!({"context": {"slot": 1}, "value": {"blockhash": Hash::new_from_array([3; 32]).to_string(), "lastValidBlockHeight": 100}})).await;
        mock(server, "sendTransaction", json!("5sigSIG")).await;
    }

    fn ix_for(kp: &Keypair) -> Vec<Instruction> {
        vec![transfer(&kp.pubkey(), &Address::from([7; 32]), 1)]
    }

    #[tokio::test]
    async fn rpc_account_null_is_none() {
        let server = MockServer::start().await;
        mock(&server, "getAccountInfo", json!({"context": {"slot": 1}, "value": null})).await;
        assert_eq!(rpc(&server).account(&Address::from([1; 32])).await.unwrap(), None);
    }

    #[tokio::test]
    async fn rpc_account_data_is_decoded() {
        let server = MockServer::start().await;
        mock(&server, "getAccountInfo", json!({"context": {"slot": 1}, "value": {"lamports": 5, "owner": "11111111111111111111111111111111", "data": ["AQID", "base64"], "executable": false, "rentEpoch": 0}})).await;
        assert_eq!(rpc(&server).account(&Address::from([1; 32])).await.unwrap(), Some(vec![1, 2, 3]));
    }

    #[tokio::test]
    async fn rpc_send_confirmed_returns_signature() {
        let server = MockServer::start().await;
        mock_send_prereqs(&server).await;
        mock(&server, "getSignatureStatuses", json!({"context": {"slot": 2}, "value": [{"slot": 2, "confirmations": 0, "err": null, "confirmationStatus": "confirmed"}]})).await;
        let kp = Keypair::new();
        assert_eq!(rpc(&server).send(ix_for(&kp), &kp).await.unwrap(), "5sigSIG");
    }

    #[tokio::test]
    async fn rpc_send_failed_status_is_rejected_with_code() {
        let server = MockServer::start().await;
        mock_send_prereqs(&server).await;
        mock(&server, "getSignatureStatuses", json!({"context": {"slot": 2}, "value": [{"slot": 2, "confirmations": 0, "err": {"InstructionError": [0, {"Custom": 6001}]}, "confirmationStatus": "confirmed"}]})).await;
        let kp = Keypair::new();
        let r = rpc(&server).send(ix_for(&kp), &kp).await;
        assert!(matches!(r, Err(ChainError::Rejected { code: Some(6001), .. })), "{r:?}");
    }

    #[tokio::test]
    async fn rpc_preflight_error_is_rejected_with_code() {
        let server = MockServer::start().await;
        mock(&server, "getLatestBlockhash", json!({"context": {"slot": 1}, "value": {"blockhash": Hash::new_from_array([3; 32]).to_string(), "lastValidBlockHeight": 100}})).await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": "sendTransaction"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32002, "message": "Transaction simulation failed", "data": {"err": {"InstructionError": [0, {"Custom": 6000}]}}}})))
            .mount(&server)
            .await;
        let kp = Keypair::new();
        let r = rpc(&server).send(ix_for(&kp), &kp).await;
        assert!(matches!(r, Err(ChainError::Rejected { code: Some(6000), .. })), "{r:?}");
    }

    async fn preflight_error(server: &MockServer, error: Value) {
        mock(server, "getLatestBlockhash", json!({"context": {"slot": 1}, "value": {"blockhash": Hash::new_from_array([3; 32]).to_string(), "lastValidBlockHeight": 100}})).await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": "sendTransaction"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": 1, "error": error})))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn rpc_error_without_data_err_is_unavailable() {
        let server = MockServer::start().await;
        preflight_error(&server, json!({"code": -32005, "message": "Node is behind"})).await;
        let kp = Keypair::new();
        let r = rpc(&server).send(ix_for(&kp), &kp).await;
        assert!(matches!(r, Err(ChainError::Unavailable(_))), "{r:?}");
    }

    #[tokio::test]
    async fn rpc_preflight_blockhash_not_found_is_unavailable() {
        let server = MockServer::start().await;
        preflight_error(&server, json!({"code": -32002, "message": "Blockhash not found", "data": {"err": "BlockhashNotFound"}})).await;
        let kp = Keypair::new();
        let r = rpc(&server).send(ix_for(&kp), &kp).await;
        assert!(matches!(r, Err(ChainError::Unavailable(_))), "{r:?}");
    }

    #[tokio::test]
    async fn rpc_other_transaction_error_has_no_code() {
        let server = MockServer::start().await;
        mock_send_prereqs(&server).await;
        mock(&server, "getSignatureStatuses", json!({"context": {"slot": 2}, "value": [{"err": "InsufficientFundsForFee", "confirmationStatus": "confirmed"}]})).await;
        let kp = Keypair::new();
        let r = rpc(&server).send(ix_for(&kp), &kp).await;
        assert!(matches!(r, Err(ChainError::Rejected { code: None, .. })), "{r:?}");
    }

    #[tokio::test]
    async fn rpc_never_confirmed_is_unavailable() {
        let server = MockServer::start().await;
        mock_send_prereqs(&server).await;
        mock(&server, "getSignatureStatuses", json!({"context": {"slot": 2}, "value": [null]})).await;
        let kp = Keypair::new();
        let r = rpc(&server).send(ix_for(&kp), &kp).await;
        assert!(matches!(r, Err(ChainError::Unavailable(_))), "{r:?}");
    }

    #[tokio::test]
    async fn rpc_http_500_is_unavailable() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let chain = rpc(&server);
        assert!(matches!(chain.account(&Address::from([1; 32])).await, Err(ChainError::Unavailable(_))));
        let kp = Keypair::new();
        assert!(matches!(chain.send(ix_for(&kp), &kp).await, Err(ChainError::Unavailable(_))));
    }
}
