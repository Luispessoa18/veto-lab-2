use aval_svm::anchor_batcher::{escaped, give_up_on_chain_errors, read_keypair, BatchError, Batcher, Paths};
use aval_svm::chain::{ChainError, CrossCheckChain, RpcChain};
use aval_svm::verify::{render, verify_line, Verdict};
use aval_svm::source::AccountSource;
use aval_svm::upstream::redact_url;
use aval_svm::{quorum::QuorumSource, cache::Cache, config::Config, engine::Engine, http::{router, App}, pool::Pool, upstream::Upstream};
use clap::{Parser, Subcommand};
use solana_address::Address;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "aval-svm", version, about = "Local Solana VM for Aval")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the RPC-compatible server.
    Serve {
        #[arg(long, default_value = "svm/aval-svm.toml")]
        config: PathBuf,
    },
    /// Print the message digest of a base64 transaction (same value Veto computes).
    Digest { tx: String },
    /// Compare aval-svm with upstream RPC on real transactions from one block.
    Shadow {
        #[arg(long, env = "AVAL_UPSTREAM_URL")]
        upstream: String,
        #[arg(long, default_value_t = 200)]
        count: usize,
        #[arg(long)]
        slot: Option<u64>,
        #[arg(long, default_value = "results/aval_shadow.jsonl")]
        out: PathBuf,
        /// Pause between transactions, to stay under public-RPC rate limits.
        #[arg(long, default_value_t = 150)]
        delay_ms: u64,
    },
    /// Anchor Merkle roots of new record lines in the aval_registry program.
    Anchor {
        #[arg(long)]
        records: PathBuf,
        /// Solana CLI keypair file of the registry authority (never printed).
        #[arg(long)]
        keypair: PathBuf,
        #[arg(long, env = "AVAL_REGISTRY_RPC", default_value = DEFAULT_UPSTREAM)]
        upstream: String,
        #[arg(long, default_value_t = 30)]
        interval_secs: u64,
        #[arg(long, default_value_t = 256)]
        max_batch: usize,
        /// Anchor what is pending, then exit.
        #[arg(long)]
        once: bool,
    },
    /// Prove one record line against the batch roots on chain.
    Verify {
        #[arg(long)]
        records: PathBuf,
        #[arg(long)]
        line: u64,
        /// Defaults to <records>.proofs.jsonl.
        #[arg(long)]
        proofs: Option<PathBuf>,
        #[arg(long, env = "AVAL_REGISTRY_RPC", default_value = DEFAULT_UPSTREAM)]
        upstream: String,
        /// Registry authority pubkey; pins the registry instead of trusting the proofs file.
        #[arg(long)]
        authority: Option<String>,
        /// Second, independent RPC: every account read must match on both, or verify exits 2.
        #[arg(long, env = "AVAL_REGISTRY_RPC_2")]
        cross_check: Option<String>,
    },
}

const DEFAULT_UPSTREAM: &str = "https://api.devnet.solana.com";
const UPSTREAM_TIMEOUT_MS: u64 = 30_000;

fn rpc_chain(url: &str) -> RpcChain {
    RpcChain::new(Upstream::new(url, "confirmed", UPSTREAM_TIMEOUT_MS))
}

/// Inclusive line range: a batch of `count` records starting at line `a` ends at `a + count - 1`.
fn anchored_message(index: u64, first_line: u64, count: u32, tx: &str) -> String {
    let last = first_line + u64::from(count).saturating_sub(1);
    format!("anchored batch {index}: lines {first_line}–{last} ({count} records) tx {tx}")
}

async fn anchor(records: PathBuf, keypair: PathBuf, upstream: String, interval_secs: u64, max_batch: usize, once: bool) -> anyhow::Result<()> {
    let signer = read_keypair(&keypair)?;
    let interval = Duration::from_secs(interval_secs.max(1));
    let mut b = Batcher::new(rpc_chain(&upstream), signer, Paths::for_records(&records), max_batch);
    let mut errors = 0u32;
    // Logs a chain error and waits, or exits 2 once `--once` has seen 3 in a row.
    let chain_error = |errors: &mut u32, e: ChainError| {
        *errors += 1;
        if give_up_on_chain_errors(once, *errors) {
            eprintln!("error: giving up after {errors} consecutive chain errors: {}", escaped(&e.to_string()));
            std::process::exit(2);
        }
        eprintln!("chain error (retrying in {}s): {}", interval.as_secs(), escaped(&e.to_string()));
    };
    loop {
        match b.startup().await {
            Ok(()) => break,
            Err(BatchError::Chain(e)) => {
                chain_error(&mut errors, e);
                tokio::time::sleep(interval).await;
            }
            Err(e) => fatal(e),
        }
    }
    errors = 0;
    eprintln!("anchoring {} into registry {} via {}", records.display(), b.registry(), redact_url(&upstream));
    loop {
        match b.anchor_pending().await {
            Ok(Some(a)) => {
                errors = 0;
                println!("{}", anchored_message(a.index, a.first_line, a.count, &a.tx));
                // A full batch means more may be waiting; otherwise batch up for one interval.
                if once || a.count as usize >= max_batch {
                    continue;
                }
                tokio::time::sleep(interval).await;
            }
            Ok(None) if once => break,
            Ok(None) => {
                errors = 0;
                tokio::time::sleep(interval).await;
            }
            Err(BatchError::Chain(e)) => {
                chain_error(&mut errors, e);
                tokio::time::sleep(interval).await;
            }
            Err(e) => fatal(e),
        }
    }
    Ok(())
}

/// Exit codes: 1 = NOT VERIFIED; 2 = could not run or check (I/O, RPC, refused to anchor).
fn fatal(e: impl std::fmt::Display) -> ! {
    eprintln!("error: {e}");
    std::process::exit(2);
}

/// Any chain read that fails or is not trusted is an `Err` here, which `main` reports through `fatal` (exit 2).
async fn check<C: aval_svm::chain::Chain>(chain: &C, records: &std::path::Path, proofs: &std::path::Path, line: u64, authority: Option<&Address>) -> anyhow::Result<Verdict> {
    Ok(verify_line(chain, records, proofs, line, authority).await?)
}

/// Exit code for a finished check: 0 verified, 1 not verified, 2 pending or could not check.
fn exit_code(r: &anyhow::Result<Verdict>) -> i32 {
    match r {
        Ok(Verdict::Verified { .. }) => 0,
        Ok(Verdict::NotVerified(_)) => 1,
        Ok(Verdict::Pending(_)) | Err(_) => 2,
    }
}

async fn verify(records: PathBuf, line: u64, proofs: Option<PathBuf>, upstream: String, authority: Option<String>, cross_check: Option<String>) -> anyhow::Result<()> {
    let authority = authority.map(|a| Address::from_str(&a).map_err(|_| anyhow::anyhow!("--authority {a:?} is not a valid pubkey"))).transpose()?;
    let proofs = proofs.unwrap_or_else(|| Paths::for_records(&records).proofs);
    let verdict = match cross_check {
        Some(second) => {
            eprintln!("note: cross-checked against a second RPC");
            let chain = CrossCheckChain::new(rpc_chain(&upstream).finalized_reads(), rpc_chain(&second).finalized_reads());
            check(&chain, &records, &proofs, line, authority.as_ref()).await?
        }
        None => check(&rpc_chain(&upstream).finalized_reads(), &records, &proofs, line, authority.as_ref()).await?,
    };
    println!("{}", render(&verdict));
    match verdict {
        Verdict::Verified { registry, .. } => {
            if authority.is_none() {
                eprintln!("note: registry {registry} was taken from the proofs file; pass --authority <pubkey> to pin it");
            }
            Ok(())
        }
        Verdict::NotVerified(_) | Verdict::Pending(_) => std::process::exit(exit_code(&Ok(verdict))),
    }
}

const DEFAULT_PRELOAD: [&str; 4] = [
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
    "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    match Cli::parse().cmd {
        Cmd::Digest { tx } => {
            let d = aval_svm::decode::decode(&tx, aval_svm::decode::Encoding::Base64)?;
            println!("{}", d.digest);
        }
        Cmd::Anchor { records, keypair, upstream, interval_secs, max_batch, once } => {
            anchor(records, keypair, upstream, interval_secs, max_batch, once).await.unwrap_or_else(|e| fatal(e))
        }
        Cmd::Verify { records, line, proofs, upstream, authority, cross_check } => verify(records, line, proofs, upstream, authority, cross_check).await.unwrap_or_else(|e| fatal(e)),
        Cmd::Shadow { upstream, count, slot, out, delay_ms } => aval_svm::shadow::run(&upstream, count, slot, &out, delay_ms).await?,
        Cmd::Serve { config } => {
            let c = Config::load(Some(&config))?;
            let upstream = Upstream::new(&c.upstream_url, &c.commitment, c.upstream_timeout_ms);
            let quorum = QuorumSource {
                primary: upstream.clone(),
                secondary: c.upstream_secondary_url.as_deref().map(|u| Upstream::new(u, &c.commitment, c.upstream_timeout_ms)),
                max_slot_gap: c.quorum_max_slot_gap,
            };
            let engine = Engine::new(Cache::new(quorum, Duration::from_millis(c.cache_ttl_ms)).with_program_ttl(Duration::from_millis(c.program_ttl_ms)), Pool::new(c.pool_size, c.recycle_after));
            let preload: Vec<Address> = DEFAULT_PRELOAD.iter().map(|s| s.to_string()).chain(c.preload_programs.clone())
                .filter_map(|s| Address::from_str(&s).ok()).collect();
            if let Err(e) = engine.cache().get_many(&preload, false).await {
                tracing::warn!("preload failed, programs load on first use: {e}");
            }
            let listener = tokio::net::TcpListener::bind(&c.listen).await?;
            println!("aval-svm on http://{} → upstream {} (pool {}, ttl {} ms)", c.listen, redact_url(&c.upstream_url), c.pool_size, c.cache_ttl_ms);
            // Never print the secondary URL: it can carry an API key.
            println!("cross-check: {}", if engine.cache().source().upstreams() == 2 { "on (2 upstreams)" } else { "off" });
            axum::serve(listener, router(Arc::new(App { engine, upstream }))).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{anchored_message, Cli, Cmd, DEFAULT_UPSTREAM};
    use clap::Parser;

    #[test]
    fn cross_check_disagreement_maps_to_exit_2() {
        use aval_svm::chain::{Chain, ChainError, CrossCheckChain};
        use solana_address::Address;
        use solana_instruction::Instruction;
        use solana_keypair::Keypair;
        struct Fake(Option<Vec<u8>>);
        impl Chain for Fake {
            async fn account(&self, _k: &Address) -> Result<Option<Vec<u8>>, ChainError> { Ok(self.0.clone()) }
            async fn send(&self, _i: Vec<Instruction>, _s: &Keypair) -> Result<String, ChainError> { unreachable!() }
        }
        let d = tempfile::tempdir().unwrap();
        let (rec, proofs) = (d.path().join("r.jsonl"), d.path().join("r.proofs.jsonl"));
        std::fs::write(&rec, "x\n").unwrap();
        std::fs::write(&proofs, "").unwrap();
        // No proof entry: a plain NOT VERIFIED is exit 1; an Err (disagreement, failed read) is 2.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let chain = CrossCheckChain::new(Fake(None), Fake(Some(vec![1])));
        let r = rt.block_on(super::check(&chain, &rec, &proofs, 0, None));
        assert_eq!(super::exit_code(&r), 1);
        assert_eq!(super::exit_code(&Err(anyhow::anyhow!("RPCs disagree on account x"))), 2);
        let pending: anyhow::Result<aval_svm::verify::Verdict> = Ok(aval_svm::verify::Verdict::Pending("p".into()));
        assert_eq!(super::exit_code(&pending), 2);
    }

    #[test]
    fn anchored_message_ends_on_the_last_line_inclusive() {
        assert_eq!(anchored_message(2, 5, 3, "SIG"), "anchored batch 2: lines 5–7 (3 records) tx SIG");
        assert_eq!(anchored_message(0, 0, 1, "SIG"), "anchored batch 0: lines 0–0 (1 records) tx SIG");
    }

    fn verify_upstream(args: &[&str]) -> String {
        let mut a = vec!["aval-svm", "verify", "--records", "r.jsonl", "--line", "0"];
        a.extend_from_slice(args);
        match Cli::try_parse_from(a).unwrap().cmd {
            Cmd::Verify { upstream, .. } => upstream,
            _ => unreachable!(),
        }
    }

    fn verify_cross_check(args: &[&str]) -> Option<String> {
        let mut a = vec!["aval-svm", "verify", "--records", "r.jsonl", "--line", "0"];
        a.extend_from_slice(args);
        match Cli::try_parse_from(a).unwrap().cmd {
            Cmd::Verify { cross_check, .. } => cross_check,
            _ => unreachable!(),
        }
    }

    #[test]
    fn cross_check_rpc_comes_from_flag_then_env_and_is_off_by_default() {
        std::env::remove_var("AVAL_REGISTRY_RPC_2");
        assert_eq!(verify_cross_check(&[]), None);
        std::env::set_var("AVAL_REGISTRY_RPC_2", "http://127.0.0.1:8998");
        assert_eq!(verify_cross_check(&[]).as_deref(), Some("http://127.0.0.1:8998"));
        assert_eq!(verify_cross_check(&["--cross-check", "http://y"]).as_deref(), Some("http://y"));
        std::env::remove_var("AVAL_REGISTRY_RPC_2");
    }

    #[test]
    fn registry_rpc_comes_from_flag_then_env_then_default() {
        std::env::remove_var("AVAL_REGISTRY_RPC");
        assert_eq!(verify_upstream(&[]), DEFAULT_UPSTREAM);
        std::env::set_var("AVAL_REGISTRY_RPC", "http://127.0.0.1:8999");
        assert_eq!(verify_upstream(&[]), "http://127.0.0.1:8999");
        assert_eq!(verify_upstream(&["--upstream", "http://x"]), "http://x");
        std::env::remove_var("AVAL_REGISTRY_RPC");
    }
}
