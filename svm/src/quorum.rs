//! Cross-checks every account read against a second RPC provider.
//!
//! Accounts are cross-checked when fetched and then served from the cache within its TTL.
//! A provider too far behind is unhealthy (error). Data that differs at different slots is
//! re-read from the lagging side (up to `refetch_attempts` times) before it counts as a
//! disagreement. A disagreement is not an error for `get_multiple_with_dissent`: the primary's
//! accounts are returned and the secondary's values ride along as dissent (the engine decides
//! whether it must refuse or simulate both worlds). `get_multiple` still refuses it.
use crate::source::{AccountSource, Dissent, SourceError};
use crate::upstream::Upstream;
use solana_account::Account;
use solana_address::Address;

type Read = (u64, Vec<Option<Account>>);

pub struct QuorumSource {
    pub primary: Upstream,
    pub secondary: Option<Upstream>,
    pub max_slot_gap: u64,
    /// Re-reads of the lagging side, on a data mismatch at different slots, before disagreeing.
    pub refetch_attempts: u32,
}

/// Indices of the keys whose two reads differ (presence, lamports, owner, executable, data).
fn differences(a: &[Option<Account>], b: &[Option<Account>]) -> Vec<usize> {
    let same = |x: &Option<Account>, y: &Option<Account>| match (x, y) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.lamports == y.lamports && x.owner == y.owner && x.executable == y.executable && x.data == y.data
        }
        _ => false,
    };
    a.iter().zip(b).enumerate().filter(|(_, (x, y))| !same(x, y)).map(|(i, _)| i).collect()
}

fn label(side: &str, e: SourceError) -> SourceError {
    SourceError::Unavailable(format!("{side} upstream unavailable: {e}"))
}

/// Both providers' final reads and the keys they still disagree on.
struct Compared {
    primary: Read,
    secondary: Read,
    differ: Vec<usize>,
}

impl QuorumSource {
    async fn compare(&self, secondary: &Upstream, keys: &[Address]) -> Result<Compared, SourceError> {
        let (p, s) = tokio::join!(self.primary.get_multiple(keys), secondary.get_multiple(keys));
        let (mut p, mut s) = (p.map_err(|e| label("primary", e))?, s.map_err(|e| label("secondary", e))?);
        // A provider too far behind is unhealthy, whatever its data says.
        if p.0.abs_diff(s.0) > self.max_slot_gap {
            let side = if p.0 < s.0 { "primary" } else { "secondary" };
            return Err(SourceError::Unavailable(format!("{side} upstream is {} slots behind", p.0.abs_diff(s.0))));
        }
        let mut differ = differences(&p.1, &s.1);
        // One side may only be behind: ask it again for state at least as new as the other's.
        // At equal slots a mismatch is a disagreement right away.
        for _ in 0..self.refetch_attempts {
            if differ.is_empty() || p.0 == s.0 {
                break;
            }
            if p.0 < s.0 {
                p = self.primary.get_multiple_at(keys, Some(s.0)).await.map_err(|e| label("primary", e))?;
            } else {
                s = secondary.get_multiple_at(keys, Some(p.0)).await.map_err(|e| label("secondary", e))?;
            }
            differ = differences(&p.1, &s.1);
        }
        Ok(Compared { primary: p, secondary: s, differ })
    }
}

impl AccountSource for QuorumSource {
    async fn get_multiple(&self, keys: &[Address]) -> Result<Read, SourceError> {
        let Some(secondary) = &self.secondary else {
            return self.primary.get_multiple(keys).await;
        };
        let c = self.compare(secondary, keys).await?;
        if let Some(&i) = c.differ.first() {
            let (p, s) = (c.primary.0, c.secondary.0);
            return Err(SourceError::Unavailable(format!("upstreams disagree on {} (slots {p}/{s})", keys[i])));
        }
        Ok((c.primary.0.max(c.secondary.0), c.primary.1))
    }

    async fn get_multiple_with_dissent(&self, keys: &[Address]) -> Result<(u64, Vec<Option<Account>>, Dissent), SourceError> {
        let Some(secondary) = &self.secondary else {
            let (slot, accounts) = self.primary.get_multiple(keys).await?;
            return Ok((slot, accounts, Dissent::new()));
        };
        let c = self.compare(secondary, keys).await?;
        let dissent = c.differ.iter().map(|&i| (keys[i], c.secondary.1[i].clone())).collect();
        Ok((c.primary.0.max(c.secondary.0), c.primary.1, dissent))
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
            refetch_attempts: 3,
        }
    }

    /// Answers the n-th call with `script[n]` (the last entry repeats): (slot, lamports of key 0).
    async fn scripted(script: Vec<(u64, u64)>) -> MockServer {
        let s = MockServer::start().await;
        let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        Mock::given(method("POST")).respond_with(move |_: &Request| {
            let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst).min(script.len() - 1);
            let (slot, lamports) = script[i];
            reply(slot, vec![acct(lamports, "AQI="), Value::Null])
        }).mount(&s).await;
        s
    }

    fn min_slots(sent: &[Value]) -> Vec<Value> {
        sent.iter().map(|b| b["params"][1].get("minContextSlot").cloned().unwrap_or(Value::Null)).collect()
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
            refetch_attempts: 3,
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

    #[tokio::test]
    async fn a_drifting_account_is_resolved_on_the_second_refetch() {
        // p 110 / s 108 differ -> s refetched (min 110) answers 111 -> p now lags, refetched (min 111) -> agree.
        let a = scripted(vec![(110, 7), (111, 8)]).await;
        let b = scripted(vec![(108, 6), (111, 8)]).await;
        let (slot, accts, dissent) = quorum(&a, Some(&b), 4).get_multiple_with_dissent(&keys()).await.unwrap();
        assert_eq!((slot, accts[0].as_ref().unwrap().lamports), (111, 8));
        assert!(dissent.is_empty());
        assert_eq!(min_slots(&bodies(&b).await), vec![Value::Null, json!(110)]);
        assert_eq!(min_slots(&bodies(&a).await), vec![Value::Null, json!(111)]);
    }

    #[tokio::test]
    async fn a_drifting_account_is_resolved_on_the_third_refetch() {
        let a = scripted(vec![(110, 7), (112, 9)]).await;
        let b = scripted(vec![(108, 6), (111, 8), (112, 9)]).await;
        let (slot, accts, dissent) = quorum(&a, Some(&b), 4).get_multiple_with_dissent(&keys()).await.unwrap();
        assert_eq!((slot, accts[0].as_ref().unwrap().lamports), (112, 9));
        assert!(dissent.is_empty());
        assert_eq!(min_slots(&bodies(&b).await), vec![Value::Null, json!(110), json!(112)]);
        assert_eq!(min_slots(&bodies(&a).await), vec![Value::Null, json!(111)]);
    }

    #[tokio::test]
    async fn still_drifting_after_every_refetch_is_dissent_not_an_error() {
        let a = scripted(vec![(110, 7), (112, 9), (114, 11)]).await;
        let b = scripted(vec![(108, 6), (111, 8), (113, 10)]).await;
        let q = quorum(&a, Some(&b), 4);
        let (slot, accts, dissent) = q.get_multiple_with_dissent(&keys()).await.unwrap();
        assert_eq!(slot, 113);
        assert_eq!(accts[0].as_ref().unwrap().lamports, 9, "the primary's latest view");
        assert_eq!(dissent.len(), 1);
        assert_eq!(dissent[&keys()[0]].as_ref().unwrap().lamports, 10, "the secondary's latest view");
        assert_eq!(bodies(&a).await.len() + bodies(&b).await.len(), 2 + 3, "three refetches at most");
    }

    #[tokio::test]
    async fn get_multiple_still_refuses_a_disagreement() {
        let a = scripted(vec![(110, 7), (112, 9), (114, 11)]).await;
        let b = scripted(vec![(108, 6), (111, 8), (113, 10)]).await;
        let e = quorum(&a, Some(&b), 4).get_multiple(&keys()).await.unwrap_err();
        let SourceError::Unavailable(m) = e else { panic!("{e:?}") };
        assert!(m.contains("disagree") && m.contains(&keys()[0].to_string()), "{m}");
    }

    #[tokio::test]
    async fn same_slot_mismatch_is_dissent_without_refetch() {
        let a = server(reply(100, vec![acct(5, "AQI="), Value::Null])).await;
        let b = server(reply(100, vec![acct(5, "AQM="), acct(1, "")])).await;
        let (_, accts, dissent) = quorum(&a, Some(&b), 4).get_multiple_with_dissent(&keys()).await.unwrap();
        assert_eq!(accts[0].as_ref().unwrap().data, vec![1, 2]);
        assert_eq!(dissent[&keys()[0]].as_ref().unwrap().data, vec![1, 3]);
        assert_eq!(dissent[&keys()[1]].as_ref().unwrap().lamports, 1, "absent on the primary, present on the secondary");
        assert_eq!(bodies(&a).await.len() + bodies(&b).await.len(), 2);
    }

    #[tokio::test]
    async fn unhealthy_provider_is_still_an_error_with_dissent() {
        let a = server(reply(110, vec![acct(7, "AQI="), Value::Null])).await;
        let b = server(reply(100, vec![acct(6, "AQI="), Value::Null])).await;
        let e = quorum(&a, Some(&b), 4).get_multiple_with_dissent(&keys()).await.unwrap_err();
        assert_eq!(e.to_string(), "upstream unavailable: secondary upstream is 10 slots behind");
    }
}
