use crate::gather::is_programdata;
use crate::source::{AccountSource, Dissent, SourceError};
use solana_account::Account;
use solana_address::Address;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const MAX_BATCH: usize = 100;
/// Default lifetime of a pinned (program / ProgramData) entry: upgrades are seen within this.
pub const PROGRAM_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Clone)]
pub struct Fetched {
    pub accounts: HashMap<Address, Option<Account>>,
    /// Newest slot over every entry returned.
    pub slot: u64,
    /// Oldest slot over the non-pinned entries returned (programs and ProgramData are
    /// allowed to be older); equals `slot` when there are none.
    pub min_slot: u64,
    /// Oldest non-pinned slot seen; `None` until one is. Never defaulted to 0.
    plain_min: Option<u64>,
    /// The secondary provider's differing values for some of `accounts` (see `QuorumSource`).
    pub dissent: Dissent,
    pub hits: usize,
    pub misses: usize,
}

struct Entry {
    account: Option<Account>,
    slot: u64,
    at: Instant,
    pinned: bool,
    /// The secondary's differing value, when the providers disagreed on this read.
    dissent: Option<Option<Account>>,
}

pub struct Cache<S> {
    source: S,
    ttl: Duration,
    program_ttl: Duration,
    map: Mutex<HashMap<Address, Entry>>,
}

impl Fetched {
    fn note_plain(&mut self, slot: u64) {
        self.plain_min = Some(self.plain_min.map_or(slot, |m| m.min(slot)));
    }

    fn settle(&mut self) {
        self.min_slot = self.plain_min.unwrap_or(self.slot);
    }

    pub fn merge(&mut self, other: Fetched) {
        // A newer read of a key replaces its dissent too.
        for k in other.accounts.keys() {
            self.dissent.remove(k);
        }
        self.dissent.extend(other.dissent);
        self.accounts.extend(other.accounts);
        self.slot = self.slot.max(other.slot);
        self.plain_min = match (self.plain_min, other.plain_min) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.settle();
        self.hits += other.hits;
        self.misses += other.misses;
    }
}

/// Programs and ProgramData are kept for `program_ttl`, unless the providers disagree on them.
fn pin(a: &Option<Account>, dissent: &Option<Option<Account>>) -> bool {
    dissent.is_none() && matches!(a, Some(a) if a.executable || is_programdata(a))
}

impl<S: AccountSource> Cache<S> {
    pub fn new(source: S, ttl: Duration) -> Self {
        Cache { source, ttl, program_ttl: PROGRAM_TTL, map: Mutex::new(HashMap::new()) }
    }

    pub fn with_program_ttl(mut self, program_ttl: Duration) -> Self {
        self.program_ttl = program_ttl;
        self
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    /// Plain accounts live `ttl`; pinned program accounts live `program_ttl` (so an upgrade
    /// is picked up). `fresh` bypasses the TTL for plain accounts only.
    pub async fn get_many(&self, keys: &[Address], fresh: bool) -> Result<Fetched, SourceError> {
        let mut out = Fetched::default();
        let mut missing = Vec::new();
        let mut seen = HashSet::new();
        {
            let map = self.map.lock().unwrap();
            for k in keys {
                if !seen.insert(*k) {
                    continue;
                }
                let live = |e: &Entry| if e.pinned { e.at.elapsed() < self.program_ttl } else { !fresh && e.at.elapsed() < self.ttl };
                match map.get(k) {
                    Some(e) if live(e) => {
                        out.accounts.insert(*k, e.account.clone());
                        if let Some(d) = &e.dissent {
                            out.dissent.insert(*k, d.clone());
                        }
                        out.slot = out.slot.max(e.slot);
                        if !e.pinned { out.note_plain(e.slot); }
                        out.hits += 1;
                    }
                    _ => missing.push(*k),
                }
            }
        }
        if missing.is_empty() {
            out.settle();
            return Ok(out);
        }
        let batches = missing.chunks(MAX_BATCH).map(|c| self.source.get_multiple_with_dissent(c));
        let results = futures::future::try_join_all(batches).await?;
        let now = Instant::now();
        let mut map = self.map.lock().unwrap();
        for (chunk, (slot, accounts, mut dissent)) in missing.chunks(MAX_BATCH).zip(results) {
            for (k, account) in chunk.iter().zip(accounts) {
                let d = dissent.remove(k);
                let pinned = pin(&account, &d);
                if !pinned { out.note_plain(slot); }
                if let Some(d) = &d {
                    out.dissent.insert(*k, d.clone());
                }
                map.insert(*k, Entry { pinned, account: account.clone(), slot, at: now, dissent: d });
                out.accounts.insert(*k, account);
            }
            out.slot = out.slot.max(slot);
        }
        out.misses += missing.len();
        out.settle();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    fn key(n: u8) -> Address { Address::from([n; 32]) }
    fn lamports(n: u64) -> Account { Account { lamports: n, ..Account::default() } }

    #[tokio::test]
    async fn second_read_is_a_hit() {
        let src = MemSource::new(10);
        src.insert(key(1), lamports(5));
        let cache = Cache::new(src, Duration::from_secs(60));
        let a = cache.get_many(&[key(1), key(2)], false).await.unwrap();
        assert_eq!((a.hits, a.misses), (0, 2));
        assert_eq!(a.accounts[&key(1)].as_ref().unwrap().lamports, 5);
        assert!(a.accounts[&key(2)].is_none()); // absent is cached too
        let b = cache.get_many(&[key(1), key(2)], false).await.unwrap();
        assert_eq!((b.hits, b.misses), (2, 0));
        assert_eq!(cache.source().calls(), 1);
    }

    #[tokio::test]
    async fn expired_and_fresh_reads_refetch_but_pinned_do_not() {
        let src = MemSource::new(10);
        src.insert(key(1), lamports(5));
        src.insert(key(2), Account { executable: true, ..lamports(1) });
        let cache = Cache::new(src, Duration::from_millis(0));
        cache.get_many(&[key(1), key(2)], false).await.unwrap();
        let again = cache.get_many(&[key(1), key(2)], true).await.unwrap();
        assert_eq!((again.hits, again.misses), (1, 1)); // executable pinned, plain refetched
    }

    #[tokio::test]
    async fn pinned_programs_are_refetched_after_program_ttl() {
        let src = MemSource::new(10);
        src.insert(key(2), Account { executable: true, ..lamports(1) });
        let cache = Cache::new(src, Duration::from_secs(60)).with_program_ttl(Duration::from_millis(30));
        cache.get_many(&[key(2)], false).await.unwrap();
        let hit = cache.get_many(&[key(2)], true).await.unwrap();
        assert_eq!((hit.hits, hit.misses), (1, 0), "within program_ttl, even fresh is a hit");
        // The program is upgraded upstream.
        cache.source().insert(key(2), Account { executable: true, ..lamports(2) });
        std::thread::sleep(Duration::from_millis(40));
        let again = cache.get_many(&[key(2)], false).await.unwrap();
        assert_eq!((again.hits, again.misses), (0, 1));
        assert_eq!(again.accounts[&key(2)].as_ref().unwrap().lamports, 2);
    }

    #[tokio::test]
    async fn duplicate_keys_are_fetched_once() {
        let src = MemSource::new(1);
        let cache = Cache::new(src, Duration::from_secs(60));
        cache.get_many(&[key(1), key(2), key(1), key(2)], false).await.unwrap();
        assert_eq!(*cache.source().batch_sizes.lock().unwrap(), vec![2]);
    }

    #[tokio::test]
    async fn batches_at_most_100_keys() {
        let src = MemSource::new(1);
        let cache = Cache::new(src, Duration::from_secs(60));
        let keys: Vec<Address> = (0..250u32).map(|i| { let mut b = [0u8; 32]; b[..4].copy_from_slice(&i.to_le_bytes()); Address::from(b) }).collect();
        cache.get_many(&keys, false).await.unwrap();
        let mut sizes = cache.source().batch_sizes.lock().unwrap().clone();
        sizes.sort();
        assert_eq!(sizes, vec![50, 100, 100]);
    }

    #[tokio::test]
    async fn upstream_failure_is_an_error_unless_everything_is_cached() {
        let src = MemSource::new(1);
        src.insert(key(1), lamports(5));
        let cache = Cache::new(src, Duration::from_secs(60));
        cache.get_many(&[key(1)], false).await.unwrap();
        cache.source().set_fail(true);
        assert!(cache.get_many(&[key(1)], false).await.is_ok());
        assert!(matches!(cache.get_many(&[key(1), key(2)], false).await, Err(SourceError::Unavailable(_))));
    }

    #[tokio::test]
    async fn min_slot_spans_hit_and_miss_ignoring_pinned() {
        let src = MemSource::new(10);
        src.insert(key(1), lamports(5));
        src.insert(key(3), Account { executable: true, ..lamports(1) });
        let cache = Cache::new(src, Duration::from_secs(60));
        let a = cache.get_many(&[key(1), key(3)], false).await.unwrap();
        assert_eq!((a.min_slot, a.slot), (10, 10));
        cache.source().set_slot(20);
        // key(1) hit at 10, key(2) miss at 20, key(3) pinned hit at 10.
        let b = cache.get_many(&[key(1), key(2), key(3)], false).await.unwrap();
        assert_eq!((b.min_slot, b.slot), (10, 20));
        let c = cache.get_many(&[key(3)], false).await.unwrap();
        assert_eq!((c.min_slot, c.slot), (10, 10), "pinned only: min == max");
    }

    #[tokio::test]
    async fn pinned_older_than_plain_does_not_lower_min() {
        let src = MemSource::new(10);
        src.insert(key(3), Account { executable: true, ..lamports(1) });
        let cache = Cache::new(src, Duration::from_secs(60));
        cache.get_many(&[key(3)], false).await.unwrap();
        cache.source().set_slot(20);
        let b = cache.get_many(&[key(3), key(4)], false).await.unwrap();
        assert_eq!((b.min_slot, b.slot), (20, 20));
    }

    #[tokio::test]
    async fn merge_with_all_pinned_side_does_not_drag_min_to_zero() {
        let src = MemSource::new(10);
        src.insert(key(1), lamports(5));
        src.insert(key(3), Account { executable: true, ..lamports(1) });
        let cache = Cache::new(src, Duration::from_secs(60));
        let mut plain = cache.get_many(&[key(1)], false).await.unwrap();
        plain.merge(cache.get_many(&[key(3)], false).await.unwrap());
        assert_eq!(plain.min_slot, 10);
        let mut pinned_first = cache.get_many(&[key(3)], false).await.unwrap();
        pinned_first.merge(cache.get_many(&[key(1)], false).await.unwrap());
        assert_eq!(pinned_first.min_slot, 10);
        let mut empty = Fetched::default();
        empty.merge(plain);
        assert_eq!(empty.min_slot, 10);
        let mut still_empty = Fetched::default();
        still_empty.merge(Fetched::default());
        assert_eq!(still_empty.min_slot, still_empty.slot);
    }

    #[tokio::test]
    async fn dissent_is_carried_on_miss_and_hit_and_expires_with_the_entry() {
        let src = MemSource::new(10);
        src.insert(key(1), lamports(5));
        src.set_dissent(key(1), Some(lamports(6)));
        let cache = Cache::new(src, Duration::from_millis(50));
        let miss = cache.get_many(&[key(1), key(2)], false).await.unwrap();
        assert_eq!(miss.accounts[&key(1)].as_ref().unwrap().lamports, 5, "the primary's view is served");
        assert_eq!(miss.dissent.len(), 1);
        assert_eq!(miss.dissent[&key(1)].as_ref().unwrap().lamports, 6);
        let hit = cache.get_many(&[key(1)], false).await.unwrap();
        assert_eq!((hit.hits, hit.dissent[&key(1)].as_ref().unwrap().lamports), (1, 6));
        // The providers now agree; once the entry expires the dissent is gone too.
        cache.source().clear_dissent();
        std::thread::sleep(Duration::from_millis(60));
        let later = cache.get_many(&[key(1)], false).await.unwrap();
        assert_eq!(later.misses, 1);
        assert!(later.dissent.is_empty());
    }

    #[tokio::test]
    async fn a_disputed_program_is_not_pinned() {
        let src = MemSource::new(10);
        src.insert(key(2), Account { executable: true, ..lamports(1) });
        src.set_dissent(key(2), None);
        let cache = Cache::new(src, Duration::from_secs(60));
        cache.get_many(&[key(2)], false).await.unwrap();
        let again = cache.get_many(&[key(2)], true).await.unwrap();
        assert_eq!(again.misses, 1, "fresh refetches a disputed program instead of keeping it for program_ttl");
    }

    #[tokio::test]
    async fn merge_carries_dissent_and_a_refetch_replaces_it() {
        let src = MemSource::new(10);
        src.insert(key(1), lamports(5));
        src.set_dissent(key(1), None);
        let cache = Cache::new(src, Duration::from_secs(60));
        let mut got = cache.get_many(&[key(1)], false).await.unwrap();
        let mut other = cache.get_many(&[key(3)], false).await.unwrap();
        other.merge(got.clone());
        assert!(other.dissent.contains_key(&key(1)));
        cache.source().clear_dissent();
        got.merge(cache.get_many(&[key(1)], true).await.unwrap());
        assert!(got.dissent.is_empty(), "the newer read of key(1) agrees");
    }
}
