//! Cross-checks every account read against a second RPC provider.
//!
//! Accounts are cross-checked when fetched and then served from the cache within its TTL.
//! Hot accounts that change every slot can make the cross-check fail more often; that is
//! fail-closed by design.
use crate::source::{AccountSource, SourceError};
use crate::upstream::Upstream;
use solana_account::Account;
use solana_address::Address;

type Read = (u64, Vec<Option<Account>>);

pub struct QuorumSource {
    pub primary: Upstream,
    pub secondary: Option<Upstream>,
    pub max_slot_gap: u64,
}

/// Index of the first key whose two reads differ (presence, lamports, owner, executable, data).
fn first_difference(a: &[Option<Account>], b: &[Option<Account>]) -> Option<usize> {
    let same = |x: &Option<Account>, y: &Option<Account>| match (x, y) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.lamports == y.lamports && x.owner == y.owner && x.executable == y.executable && x.data == y.data
        }
        _ => false,
    };
    a.iter().zip(b).position(|(x, y)| !same(x, y))
}

fn label(side: &str, e: SourceError) -> SourceError {
    SourceError::Unavailable(format!("{side} upstream unavailable: {e}"))
}

impl AccountSource for QuorumSource {
    async fn get_multiple(&self, keys: &[Address]) -> Result<Read, SourceError> {
        let Some(secondary) = &self.secondary else {
            return self.primary.get_multiple(keys).await;
        };
        let (p, s) = tokio::join!(self.primary.get_multiple(keys), secondary.get_multiple(keys));
        let (mut p, mut s) = (p.map_err(|e| label("primary", e))?, s.map_err(|e| label("secondary", e))?);
        // A provider too far behind is unhealthy, whatever its data says.
        if p.0.abs_diff(s.0) > self.max_slot_gap {
            let side = if p.0 < s.0 { "primary" } else { "secondary" };
            return Err(SourceError::Unavailable(format!("{side} upstream is {} slots behind", p.0.abs_diff(s.0))));
        }
        let Some(first) = first_difference(&p.1, &s.1) else {
            return Ok((p.0.max(s.0), p.1));
        };
        let disagree = |i: usize, p: u64, s: u64| {
            SourceError::Unavailable(format!("upstreams disagree on {} (slots {p}/{s})", keys[i]))
        };
        if p.0 == s.0 {
            return Err(disagree(first, p.0, s.0));
        }
        // One side may only be behind: ask it again for state at least as new as the other's.
        if p.0 < s.0 {
            p = self.primary.get_multiple_at(keys, Some(s.0)).await.map_err(|e| label("primary", e))?;
        } else {
            s = secondary.get_multiple_at(keys, Some(p.0)).await.map_err(|e| label("secondary", e))?;
        }
        if let Some(i) = first_difference(&p.1, &s.1) {
            return Err(disagree(i, p.0, s.0));
        }
        Ok((p.0.max(s.0), p.1))
    }

    fn upstreams(&self) -> usize {
        if self.secondary.is_some() { 2 } else { 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const OWNER: &str = "11111111111111111111111111111111";

    fn acct(lamports: u64, data: &str) -> Value {
        json!({"lamports": lamports, "owner": OWNER, "data": [data, "base64"], "executable": false, "rentEpoch": 0, "space": 0})
    }

    fn reply(slot: u64, values: Vec<Value>) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": {"context": {"slot": slot}, "value": values}}))
    }

    async fn server(r: ResponseTemplate) -> MockServer {
        let s = MockServer::start().await;
        Mock::given(method("POST")).respond_with(r).mount(&s).await;
        s
    }

    fn quorum(a: &MockServer, b: Option<&MockServer>, gap: u64) -> QuorumSource {
        QuorumSource {
            primary: Upstream::new(&a.uri(), "confirmed", 2000),
            secondary: b.map(|b| Upstream::new(&b.uri(), "confirmed", 2000)),
            max_slot_gap: gap,
        }
    }

    fn keys() -> Vec<Address> {
        vec![Address::from([1; 32]), Address::from([2; 32])]
    }

    async fn bodies(s: &MockServer) -> Vec<Value> {
        s.received_requests().await.unwrap().iter().map(|r| r.body_json().unwrap()).collect()
    }

    #[tokio::test]
    async fn agreeing_providers_return_max_slot_and_primary_accounts() {
        let a = server(reply(100, vec![acct(5, "AQI="), Value::Null])).await;
        let b = server(reply(102, vec![acct(5, "AQI="), Value::Null])).await;
        let (slot, accts) = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap();
        assert_eq!(slot, 102);
        assert_eq!(accts[0].as_ref().unwrap().lamports, 5);
        assert!(accts[1].is_none());
    }

    #[tokio::test]
    async fn same_slot_data_mismatch_names_the_account() {
        let a = server(reply(100, vec![acct(5, "AQI="), Value::Null])).await;
        let b = server(reply(100, vec![acct(5, "AQM="), Value::Null])).await;
        let e = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap_err();
        let SourceError::Unavailable(m) = e else { panic!("{e:?}") };
        assert!(m.contains(&keys()[0].to_string()) && m.contains("100/100"), "{m}");
        assert_eq!(bodies(&a).await.len(), 1, "no refetch when slots are equal");
    }

    #[tokio::test]
    async fn presence_mismatch_is_a_disagreement() {
        let a = server(reply(100, vec![acct(5, "AQI="), Value::Null])).await;
        let b = server(reply(100, vec![acct(5, "AQI="), acct(1, "")])).await;
        assert!(quorum(&a, Some(&b), 4).get_multiple(&keys()).await.is_err());
    }

    #[tokio::test]
    async fn lagging_secondary_is_refetched_with_min_context_slot() {
        let a = server(reply(110, vec![acct(7, "AQI="), Value::Null])).await;
        let b = MockServer::start().await;
        // The first read is stale; the refetch (carrying minContextSlot) has caught up.
        Mock::given(method("POST")).respond_with(|r: &Request| {
            let body: Value = r.body_json().unwrap();
            if body["params"][1].get("minContextSlot").is_some() {
                reply(111, vec![acct(7, "AQI="), Value::Null])
            } else {
                reply(108, vec![acct(6, "AQI="), Value::Null])
            }
        }).mount(&b).await;
        let (slot, accts) = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap();
        assert_eq!(slot, 111);
        assert_eq!(accts[0].as_ref().unwrap().lamports, 7);
        let sent = bodies(&b).await;
        assert_eq!(sent.len(), 2);
        assert!(sent[0]["params"][1].get("minContextSlot").is_none());
        assert_eq!(sent[1]["params"][1]["minContextSlot"], 110);
        assert_eq!(bodies(&a).await.len(), 1, "the newer side is not refetched");
    }

    #[tokio::test]
    async fn still_different_after_refetch_is_unavailable() {
        let a = server(reply(110, vec![acct(7, "AQI="), Value::Null])).await;
        // b is stale at first, then reports slot 110 but with different data.
        let b_first = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|r: &Request| {
            let body: Value = r.body_json().unwrap();
            if body["params"][1].get("minContextSlot").is_some() {
                reply(110, vec![acct(6, "AQI="), Value::Null])
            } else {
                reply(108, vec![acct(6, "AQI="), Value::Null])
            }
        }).mount(&b_first).await;
        let e = quorum(&a, Some(&b_first), 4).get_multiple(&keys()).await.unwrap_err();
        let SourceError::Unavailable(m) = e else { panic!("{e:?}") };
        assert!(m.contains("disagree") && m.contains(&keys()[0].to_string()), "{m}");
        assert_eq!(bodies(&b_first).await.len(), 2);
    }

    #[tokio::test]
    async fn secondary_beyond_the_gap_is_unhealthy_even_when_data_agrees() {
        let a = server(reply(110, vec![acct(7, "AQI="), Value::Null])).await;
        let b = server(reply(100, vec![acct(7, "AQI="), Value::Null])).await;
        let e = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap_err();
        let SourceError::Unavailable(m) = e else { panic!("{e:?}") };
        assert_eq!(m, "secondary upstream is 10 slots behind");
        assert_eq!(bodies(&b).await.len(), 1, "no refetch");
    }

    #[tokio::test]
    async fn primary_beyond_the_gap_is_unhealthy() {
        let a = server(reply(100, vec![Value::Null, Value::Null])).await;
        let b = server(reply(110, vec![Value::Null, Value::Null])).await;
        let e = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap_err();
        let SourceError::Unavailable(m) = e else { panic!("{e:?}") };
        assert_eq!(m, "primary upstream is 10 slots behind");
    }

    #[tokio::test]
    async fn lagging_primary_is_refetched_with_min_context_slot() {
        let b = server(reply(110, vec![acct(7, "AQI="), Value::Null])).await;
        let a = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|r: &Request| {
            let body: Value = r.body_json().unwrap();
            if body["params"][1].get("minContextSlot").is_some() {
                reply(111, vec![acct(7, "AQI="), Value::Null])
            } else {
                reply(108, vec![acct(6, "AQI="), Value::Null])
            }
        }).mount(&a).await;
        let (slot, accts) = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap();
        assert_eq!((slot, accts[0].as_ref().unwrap().lamports), (111, 7));
        let sent = bodies(&a).await;
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1]["params"][1]["minContextSlot"], 110);
        assert_eq!(bodies(&b).await.len(), 1);
    }

    #[tokio::test]
    async fn transport_errors_do_not_leak_the_secondary_url() {
        let a = server(reply(100, vec![Value::Null, Value::Null])).await;
        let q = QuorumSource {
            primary: Upstream::new(&a.uri(), "confirmed", 2000),
            secondary: Some(Upstream::new("http://127.0.0.1:9/?api-key=SECRET123", "confirmed", 2000)),
            max_slot_gap: 4,
        };
        let e = q.get_multiple(&keys()).await.unwrap_err().to_string();
        assert!(e.starts_with("upstream unavailable: secondary") && !e.contains("SECRET123"), "{e}");
    }

    #[tokio::test]
    async fn secondary_http_error_fails_closed() {
        let a = server(reply(100, vec![Value::Null, Value::Null])).await;
        let b = server(ResponseTemplate::new(500)).await;
        let e = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap_err();
        let SourceError::Unavailable(m) = e else { panic!("{e:?}") };
        assert!(m.starts_with("secondary upstream unavailable"), "{m}");
    }

    #[tokio::test]
    async fn primary_http_error_fails_closed() {
        let a = server(ResponseTemplate::new(500)).await;
        let b = server(reply(100, vec![Value::Null, Value::Null])).await;
        let e = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap_err();
        let SourceError::Unavailable(m) = e else { panic!("{e:?}") };
        assert!(m.starts_with("primary upstream unavailable"), "{m}");
    }

    #[tokio::test]
    async fn without_secondary_only_the_primary_is_called() {
        let a = server(reply(100, vec![acct(5, "AQI="), Value::Null])).await;
        let q = quorum(&a, None, 4);
        assert_eq!(q.upstreams(), 1);
        let (slot, _) = q.get_multiple(&keys()).await.unwrap();
        assert_eq!(slot, 100);
        assert_eq!(bodies(&a).await.len(), 1);
    }

    #[tokio::test]
    async fn two_providers_report_two_upstreams() {
        let a = server(reply(1, vec![])).await;
        assert_eq!(quorum(&a, Some(&a), 4).upstreams(), 2);
    }
}
