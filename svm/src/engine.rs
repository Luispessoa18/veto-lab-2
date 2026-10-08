use crate::cache::{Cache, Fetched};
use crate::decode::Decoded;
use crate::gather::{first_pass, is_programdata, programdata_of, resolve_lookups, GatherError};
use crate::pool::{Pool, SimError, SimInput, SimOutcome};
use crate::project::token_authorities;
use crate::source::{AccountSource, Dissent, SourceError};
use solana_account::Account;
use solana_address::Address;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Sysvars fetched with every request and applied to the VM (see `pool::run_in`).
pub const CLUSTER_SYSVARS: [Address; 3] =
    [solana_sdk_ids::sysvar::rent::ID, solana_sdk_ids::sysvar::clock::ID, solana_sdk_ids::sysvar::epoch_schedule::ID];

/// One simulated view of the chain: its input state and its outcome.
#[derive(Debug)]
pub struct World {
    pub outcome: SimOutcome,
    pub pre: HashMap<Address, Option<Account>>,
}

#[derive(Debug)]
pub struct SimReport {
    /// The answer: world P's, or the failing world's when one of two fails.
    pub outcome: SimOutcome,
    /// The input state of the world `outcome` came from.
    pub pre: HashMap<Address, Option<Account>>,
    pub slot: u64,
    /// Oldest slot among the non-pinned reads (see `Fetched::min_slot`).
    pub min_slot: u64,
    pub hits: usize,
    pub misses: usize,
    pub elapsed_us: u64,
    pub digest: String,
    /// Providers each read was checked against (1 = no cross-check).
    pub upstreams: usize,
    /// 2 when the providers disagreed on tolerated accounts and both views were simulated.
    pub worlds: usize,
    /// The two worlds' results differ (error presence, or a written account's post-state).
    pub divergent: bool,
    /// With two worlds: the one not reported in `outcome` (world S when both succeed).
    pub alternate: Option<World>,
}

/// Dissent the providers must not have: signers (fee payer included), programs and
/// ProgramData, and token accounts a signer owns or is delegate of — in either view.
/// Lookup tables too: they decide which accounts the transaction loads at all.
fn strict_dissent(decoded: &Decoded, accounts: &HashMap<Address, Option<Account>>, dissent: &Dissent) -> Option<Address> {
    let msg = &decoded.tx.message;
    let signers: HashSet<Address> =
        msg.static_account_keys().iter().take(msg.header().num_required_signatures as usize).copied().collect();
    let tables: HashSet<Address> = msg.address_table_lookups().unwrap_or(&[]).iter().map(|l| l.account_key).collect();
    let strict_view = |a: &Option<Account>| match a {
        Some(a) => a.executable || is_programdata(a) || token_authorities(a).is_some_and(|(owner, delegate)| {
            signers.contains(&owner) || delegate.is_some_and(|d| signers.contains(&d))
        }),
        None => false,
    };
    let mut strict: Vec<Address> = dissent.iter()
        .filter(|(k, theirs)| {
            signers.contains(*k) || tables.contains(*k) || strict_view(theirs)
                || accounts.get(*k).is_some_and(strict_view)
        })
        .map(|(k, _)| *k)
        .collect();
    strict.sort_by_key(|k| k.to_string());
    strict.first().copied()
}

fn same_account(a: &Account, b: &Account) -> bool {
    a.lamports == b.lamports && a.owner == b.owner && a.executable == b.executable && a.data == b.data
}

/// The two worlds' results differ: one failed and the other did not, or an account written in
/// one has a different (or no) post-state in the other.
fn diverges(p: &SimOutcome, s: &SimOutcome) -> bool {
    p.err.is_some() != s.err.is_some()
        || p.post.len() != s.post.len()
        || p.post.iter().any(|(k, a)| s.post.get(k).is_none_or(|b| !same_account(a, b)))
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("upstream unavailable: {0}")]
    Upstream(String),
    #[error("{0}")]
    Gather(#[from] GatherError),
    #[error("unsupported program {0}")]
    Unsupported(Address),
    #[error("internal: {0}")]
    Internal(String),
}

impl EngineError {
    pub fn code(&self) -> i64 {
        match self {
            EngineError::Upstream(_) => -32005,
            EngineError::Gather(_) => -32602,
            EngineError::Unsupported(_) => -32004,
            EngineError::Internal(_) => -32603,
        }
    }
}

impl From<SourceError> for EngineError {
    fn from(e: SourceError) -> Self { EngineError::Upstream(e.to_string()) }
}

impl From<SimError> for EngineError {
    fn from(e: SimError) -> Self {
        match e {
            SimError::UnsupportedProgram(k) => EngineError::Unsupported(k),
            SimError::PoolDown | SimError::Panicked => EngineError::Internal(e.to_string()),
        }
    }
}

pub struct Engine<S> {
    cache: Cache<S>,
    pool: Pool,
}

impl<S: AccountSource> Engine<S> {
    pub fn new(cache: Cache<S>, pool: Pool) -> Self { Engine { cache, pool } }

    pub fn cache(&self) -> &Cache<S> { &self.cache }

    async fn gather(&self, decoded: &Decoded, fresh: bool) -> Result<Fetched, EngineError> {
        let msg = &decoded.tx.message;
        // The cluster's sysvars ride along: rent differs from LiteSVM's default, and programs
        // read the cluster's time and epoch from Clock.
        let mut keys = first_pass(msg);
        keys.extend(CLUSTER_SYSVARS);
        let mut got = self.cache.get_many(&keys, fresh).await?;
        let loaded = match resolve_lookups(msg, &got.accounts) {
            Ok(loaded) => loaded,
            // A table created or extended after it was cached: re-read the tables once.
            Err(_) => {
                let tables: Vec<Address> =
                    msg.address_table_lookups().unwrap_or(&[]).iter().map(|l| l.account_key).collect();
                got.merge(self.cache.get_many(&tables, true).await?);
                resolve_lookups(msg, &got.accounts)?
            }
        };
        if !loaded.is_empty() {
            got.merge(self.cache.get_many(&loaded, fresh).await?);
        }
        let pds: Vec<Address> = got.accounts.values().flatten().filter_map(programdata_of).collect();
        if !pds.is_empty() {
            got.merge(self.cache.get_many(&pds, fresh).await?);
        }
        Ok(got)
    }

    pub async fn simulate(&self, decoded: Decoded, fresh: bool) -> Result<SimReport, EngineError> {
        let started = Instant::now();
        let mut got = self.gather(&decoded, fresh).await?;
        if let Some(k) = strict_dissent(&decoded, &got.accounts, &got.dissent) {
            return Err(EngineError::Upstream(format!("upstreams disagree on {k}")));
        }
        let slot = got.slot;
        let input = |tx, accounts: &HashMap<Address, Option<Account>>| SimInput {
            tx,
            accounts: accounts.iter().map(|(k, a)| (*k, a.clone())).collect(),
            slot,
        };
        let (outcome, pre, worlds, divergent, alternate) = if got.dissent.is_empty() {
            let outcome = self.pool.run(input(decoded.tx, &got.accounts)).await?;
            (outcome, std::mem::take(&mut got.accounts), 1, false, None)
        } else {
            // Tolerated dissent (pools, oracles, sysvars): simulate the primary's view (P) and
            // the primary's view with the secondary's values (S); report the stricter.
            let mut theirs = got.accounts.clone();
            theirs.extend(got.dissent.iter().map(|(k, a)| (*k, a.clone())));
            let (p, s) = tokio::join!(
                self.pool.run(input(decoded.tx.clone(), &got.accounts)),
                self.pool.run(input(decoded.tx, &theirs)),
            );
            let p = World { outcome: p?, pre: std::mem::take(&mut got.accounts) };
            let s = World { outcome: s?, pre: theirs };
            let divergent = diverges(&p.outcome, &s.outcome);
            let (shown, other) = if p.outcome.err.is_none() && s.outcome.err.is_some() { (s, p) } else { (p, s) };
            (shown.outcome, shown.pre, 2, divergent, Some(other))
        };
        Ok(SimReport {
            outcome,
            pre,
            slot: got.slot,
            min_slot: got.min_slot,
            hits: got.hits,
            misses: got.misses,
            elapsed_us: started.elapsed().as_micros() as u64,
            digest: decoded.digest,
            upstreams: self.cache.source().upstreams(),
            worlds,
            divergent,
            alternate,
        })
    }
}
