use solana_account::Account;
use solana_address::Address;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, thiserror::Error)]
pub enum SourceError {
    #[error("upstream unavailable: {0}")]
    Unavailable(String),
    #[error("upstream rpc error: {0}")]
    Rpc(serde_json::Value),
}

/// A second provider's differing view of some accounts: address -> its value (None = absent).
pub type Dissent = HashMap<Address, Option<Account>>;

pub trait AccountSource: Send + Sync + 'static {
    /// Accounts in the same order as `keys`, plus the slot they were read at.
    fn get_multiple(
        &self,
        keys: &[Address],
    ) -> impl Future<Output = Result<(u64, Vec<Option<Account>>), SourceError>> + Send;

    /// Like `get_multiple`, but a provider disagreement is not an error: the primary's accounts
    /// are returned and the secondary's differing values come back as dissent. Default: none.
    fn get_multiple_with_dissent(
        &self,
        keys: &[Address],
    ) -> impl Future<Output = Result<(u64, Vec<Option<Account>>, Dissent), SourceError>> + Send {
        async move {
            let (slot, accounts) = self.get_multiple(keys).await?;
            Ok((slot, accounts, Dissent::new()))
        }
    }

    /// How many independent upstreams every read is checked against.
    fn upstreams(&self) -> usize {
        1
    }
}

/// In-memory source for tests and fixture replay. Not used by `serve`.
pub struct MemSource {
    pub accounts: Mutex<HashMap<Address, Account>>,
    slot: AtomicU64,
    calls: AtomicUsize,
    pub batch_sizes: Mutex<Vec<usize>>,
    fail: AtomicBool,
    /// A pretend secondary provider's differing values (see `set_dissent`).
    secondary: Mutex<Dissent>,
}

impl MemSource {
    pub fn new(slot: u64) -> Self {
        MemSource { accounts: Mutex::default(), slot: AtomicU64::new(slot), calls: AtomicUsize::new(0), batch_sizes: Mutex::default(), fail: AtomicBool::new(false), secondary: Mutex::default() }
    }
    pub fn insert(&self, k: Address, a: Account) {
        self.accounts.lock().unwrap().insert(k, a);
    }
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    /// Moves the slot later reads are reported at (tests).
    pub fn set_slot(&self, slot: u64) {
        self.slot.store(slot, Ordering::SeqCst);
    }
    pub fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }
    /// Makes a pretend secondary provider report `secondary` for `k` (tests of the two worlds).
    pub fn set_dissent(&self, k: Address, secondary: Option<Account>) {
        self.secondary.lock().unwrap().insert(k, secondary);
    }
    pub fn clear_dissent(&self) {
        self.secondary.lock().unwrap().clear();
    }
}

impl AccountSource for MemSource {
    async fn get_multiple(&self, keys: &[Address]) -> Result<(u64, Vec<Option<Account>>), SourceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.batch_sizes.lock().unwrap().push(keys.len());
        if self.fail.load(Ordering::SeqCst) {
            return Err(SourceError::Unavailable("mem source set to fail".into()));
        }
        let map = self.accounts.lock().unwrap();
        Ok((self.slot.load(Ordering::SeqCst), keys.iter().map(|k| map.get(k).cloned()).collect()))
    }

    async fn get_multiple_with_dissent(&self, keys: &[Address]) -> Result<(u64, Vec<Option<Account>>, Dissent), SourceError> {
        let (slot, accounts) = self.get_multiple(keys).await?;
        let secondary = self.secondary.lock().unwrap();
        let dissent = keys.iter().filter_map(|k| secondary.get(k).map(|a| (*k, a.clone()))).collect();
        Ok((slot, accounts, dissent))
    }
}
