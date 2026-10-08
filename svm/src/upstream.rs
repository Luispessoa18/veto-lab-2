use crate::source::{AccountSource, SourceError};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};
use solana_account::Account;
use solana_address::Address;
use std::str::FromStr;
use std::time::Duration;

/// `scheme://host[:port]` of a URL, for logs: path, query and credentials (API keys) are dropped.
pub fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return "<url>".into() };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    format!("{scheme}://{host}")
}

#[derive(Clone)]
pub struct Upstream {
    url: String,
    commitment: String,
    client: reqwest::Client,
}

fn parse_account(v: &Value) -> Result<Option<Account>, SourceError> {
    if v.is_null() {
        return Ok(None);
    }
    let bad = |what: &str| SourceError::Unavailable(format!("malformed account from upstream: {what}"));
    let data = v["data"][0].as_str().ok_or_else(|| bad("data"))?;
    Ok(Some(Account {
        lamports: v["lamports"].as_u64().ok_or_else(|| bad("lamports"))?,
        data: B64.decode(data).map_err(|_| bad("base64"))?,
        owner: Address::from_str(v["owner"].as_str().ok_or_else(|| bad("owner"))?).map_err(|_| bad("owner"))?,
        executable: v["executable"].as_bool().unwrap_or(false),
        rent_epoch: v["rentEpoch"].as_u64().unwrap_or(u64::MAX),
    }))
}

impl Upstream {
    pub fn new(url: &str, commitment: &str, timeout_ms: u64) -> Upstream {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .expect("reqwest client");
        Upstream { url: url.to_string(), commitment: commitment.to_string(), client }
    }

    /// Sends a raw JSON-RPC body and returns the raw reply body.
    pub async fn forward(&self, body: &Value) -> Result<Value, SourceError> {
        let resp = self
            .client
            .post(&self.url)
            .json(body)
            .send()
            .await
            .map_err(|e| SourceError::Unavailable(e.without_url().to_string()))?;
        if !resp.status().is_success() {
            return Err(SourceError::Unavailable(format!("HTTP {}", resp.status())));
        }
        resp.json().await.map_err(|e| SourceError::Unavailable(e.without_url().to_string()))
    }

    /// One call; returns `result`, or `SourceError::Rpc(error)`.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, SourceError> {
        let body = self.forward(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).await?;
        if let Some(err) = body.get("error") {
            return Err(SourceError::Rpc(err.clone()));
        }
        Ok(body["result"].clone())
    }
}

impl Upstream {
    /// `getMultipleAccounts`; with `min_context_slot` the node must answer from at least that slot.
    pub async fn get_multiple_at(
        &self,
        keys: &[Address],
        min_context_slot: Option<u64>,
    ) -> Result<(u64, Vec<Option<Account>>), SourceError> {
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        let mut config = json!({"encoding": "base64", "commitment": self.commitment});
        if let Some(slot) = min_context_slot {
            config["minContextSlot"] = json!(slot);
        }
        let result = self
            .call("getMultipleAccounts", json!([keys, config]))
            .await
            .map_err(|e| SourceError::Unavailable(e.to_string()))?;
        let slot = result["context"]["slot"]
            .as_u64()
            .ok_or_else(|| SourceError::Unavailable("missing context slot".into()))?;
        let values = result["value"].as_array().ok_or(SourceError::Unavailable("no value array".into()))?;
        if values.len() != keys.len() {
            return Err(SourceError::Unavailable(format!(
                "upstream returned {} accounts for {} keys",
                values.len(),
                keys.len()
            )));
        }
        let accounts = values.iter().map(parse_account).collect::<Result<Vec<_>, _>>()?;
        Ok((slot, accounts))
    }
}

impl AccountSource for Upstream {
    async fn get_multiple(&self, keys: &[Address]) -> Result<(u64, Vec<Option<Account>>), SourceError> {
        self.get_multiple_at(keys, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn redact_url_keeps_only_scheme_and_host() {
        assert_eq!(redact_url("https://mainnet.helius-rpc.com/?api-key=SECRET123"), "https://mainnet.helius-rpc.com");
        assert_eq!(redact_url("http://user:SECRET123@127.0.0.1:8899/path?x=1#f"), "http://127.0.0.1:8899");
        assert_eq!(redact_url("not a url SECRET123"), "<url>");
    }

    #[tokio::test]
    async fn parses_get_multiple_accounts() {
        let server = MockServer::start().await;
        let owner = "11111111111111111111111111111111";
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": "getMultipleAccounts"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 1,
                "result": {"context": {"slot": 77}, "value": [
                    {"lamports": 5, "owner": owner, "data": ["AQI=", "base64"], "executable": false, "rentEpoch": 18446744073709551615u64, "space": 2},
                    null
                ]}
            })))
            .mount(&server).await;
        let up = Upstream::new(&server.uri(), "confirmed", 2000);
        let (slot, accts) = up.get_multiple(&[Address::from([1; 32]), Address::from([2; 32])]).await.unwrap();
        assert_eq!(slot, 77);
        let a = accts[0].as_ref().unwrap();
        assert_eq!((a.lamports, a.data.clone(), a.rent_epoch), (5, vec![1, 2], u64::MAX));
        assert!(accts[1].is_none());
    }

    #[tokio::test]
    async fn http_failure_is_unavailable() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(429)).mount(&server).await;
        let up = Upstream::new(&server.uri(), "confirmed", 2000);
        assert!(matches!(up.get_multiple(&[Address::from([1; 32])]).await, Err(SourceError::Unavailable(_))));
    }

    async fn reply_with(result: Value) -> Result<(u64, Vec<Option<Account>>), SourceError> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": result})))
            .mount(&server).await;
        let up = Upstream::new(&server.uri(), "confirmed", 2000);
        up.get_multiple(&[Address::from([1; 32]), Address::from([2; 32])]).await
    }

    #[tokio::test]
    async fn short_value_array_is_unavailable() {
        let r = reply_with(json!({"context": {"slot": 1}, "value": [null]})).await;
        assert!(matches!(r, Err(SourceError::Unavailable(_))));
    }

    #[tokio::test]
    async fn missing_slot_is_unavailable() {
        let r = reply_with(json!({"context": {}, "value": [null, null]})).await;
        assert!(matches!(r, Err(SourceError::Unavailable(_))));
    }
}
