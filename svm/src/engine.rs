use crate::cache::{Cache, Fetched};
use crate::decode::Decoded;
use crate::gather::{first_pass, is_programdata, programdata_of, resolve_lookups, GatherError};
use crate::pool::{Pool, SimError, SimInput, SimOutcome};
use crate::project::{divergent, project, token_authorities, WorldView};
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
    /// The two worlds differ in what matters to the user (see `project::divergent`).
    pub divergent: bool,
    /// With two worlds: the one not reported in `outcome` (world S when both succeed).
    pub alternate: Option<World>,
}

/// Dissent the providers must not have: signers (fee payer included), programs and
/// ProgramData, and token accounts a signer owns or is delegate of — in either view.
/// Lookup tables too: they decide which accounts the transaction loads at all.
fn strict_dissent(decoded: &Decoded, accounts: &HashMap<Address, Option<Account>>, dissent: &Dissent) -> Option<Address> {
    let msg = &decoded.tx.message;
    let signers = signers_of(decoded);
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

fn signers_of(decoded: &Decoded) -> HashSet<Address> {
    let msg = &decoded.tx.message;
    msg.static_account_keys().iter().take(msg.header().num_required_signatures as usize).copied().collect()
}

/// Divergence between two simulated worlds, over their projections (see `project::divergent`).
fn diverges(p: &World, s: &World, signers: &HashSet<Address>, tolerance_bps: u64) -> bool {
    // Token accounts the signers own or are delegate of, in any view before or after.
    let user = |a: &Account| token_authorities(a).is_some_and(|(o, d)| signers.contains(&o) || d.is_some_and(|d| signers.contains(&d)));
    let user_tokens: HashSet<String> = [p, s].iter()
        .flat_map(|w| w.pre.iter().filter_map(|(k, a)| a.as_ref().map(|a| (k, a))).chain(w.outcome.post.iter()))
        .filter(|(_, a)| user(a))
        .map(|(k, _)| k.to_string())
        .collect();
    let names: HashSet<String> = signers.iter().map(|k| k.to_string()).collect();
    let (pp, sp) = (project(&p.pre, &p.outcome.post), project(&s.pre, &s.outcome.post));
    divergent(WorldView { err: p.outcome.err.as_ref(), projection: &pp },
              WorldView { err: s.outcome.err.as_ref(), projection: &sp }, &names, &user_tokens, tolerance_bps)
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

/// Default `divergence_tolerance_bps`.
pub const DIVERGENCE_TOLERANCE_BPS: u64 = 50;

pub struct Engine<S> {
    cache: Cache<S>,
    pool: Pool,
    tolerance_bps: u64,
}

impl<S: AccountSource> Engine<S> {
    pub fn new(cache: Cache<S>, pool: Pool) -> Self { Engine { cache, pool, tolerance_bps: DIVERGENCE_TOLERANCE_BPS } }

    /// How far (in bps of the larger delta) the user's deltas may differ between two worlds.
    pub fn with_divergence_tolerance_bps(mut self, bps: u64) -> Self {
        self.tolerance_bps = bps;
        self
    }

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
        let signers = signers_of(&decoded);
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
            let divergent = diverges(&p, &s, &signers, self.tolerance_bps);
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
