use crate::cache::{Cache, Fetched};
use crate::decode::Decoded;
use crate::gather::{first_pass, programdata_of, resolve_lookups, GatherError};
use crate::pool::{Pool, SimError, SimInput, SimOutcome};
use crate::source::{AccountSource, SourceError};
use solana_account::Account;
use solana_address::Address;
use std::collections::HashMap;
use std::time::Instant;

/// Sysvars fetched with every request and applied to the VM (see `pool::run_in`).
pub const CLUSTER_SYSVARS: [Address; 3] =
    [solana_sdk_ids::sysvar::rent::ID, solana_sdk_ids::sysvar::clock::ID, solana_sdk_ids::sysvar::epoch_schedule::ID];

#[derive(Debug)]
pub struct SimReport {
    pub outcome: SimOutcome,
    pub pre: HashMap<Address, Option<Account>>,
    pub slot: u64,
    pub hits: usize,
    pub misses: usize,
    pub elapsed_us: u64,
    pub digest: String,
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
        let got = self.gather(&decoded, fresh).await?;
        let input = SimInput {
            tx: decoded.tx,
            accounts: got.accounts.iter().map(|(k, a)| (*k, a.clone())).collect(),
            slot: got.slot,
        };
        let outcome = self.pool.run(input).await?;
        Ok(SimReport {
            outcome,
            pre: got.accounts,
            slot: got.slot,
            hits: got.hits,
            misses: got.misses,
            elapsed_us: started.elapsed().as_micros() as u64,
            digest: decoded.digest,
        })
    }
}
