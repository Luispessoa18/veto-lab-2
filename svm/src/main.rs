use aval_svm::anchor_batcher::{escaped, give_up_on_chain_errors, read_keypair, BatchError, Batcher, Paths};
use aval_svm::chain::{ChainError, RpcChain};
use aval_svm::verify::{registry_authority, render, verify_line, Verdict};
use aval_svm::{cache::Cache, config::Config, engine::Engine, http::{router, App}, pool::Pool, upstream::Upstream};
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
        #[arg(long, default_value = DEFAULT_UPSTREAM)]
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
        #[arg(long, default_value = DEFAULT_UPSTREAM)]
        upstream: String,
        /// Registry authority pubkey; pins the registry instead of trusting the proofs file.
        #[arg(long)]
        authority: Option<String>,
    },
}

const DEFAULT_UPSTREAM: &str = "https://api.devnet.solana.com";
const UPSTREAM_TIMEOUT_MS: u64 = 30_000;

fn rpc_chain(url: &str) -> RpcChain {
    RpcChain::new(Upstream::new(url, "confirmed", UPSTREAM_TIMEOUT_MS))
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
            eprintln!("error: giving up after {errors} consecutive chain errors: {e}");
            std::process::exit(2);
        }
        eprintln!("chain error (retrying in {}s): {e}", interval.as_secs());
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
    eprintln!("anchoring {} into registry {} via {upstream}", records.display(), b.registry());
    loop {
        match b.anchor_pending().await {
            Ok(Some(a)) => {
                errors = 0;
                println!("anchored batch {}: lines {}..{} tx {}", a.index, a.first_line, a.first_line + a.count as u64, a.tx);
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

async fn verify(records: PathBuf, line: u64, proofs: Option<PathBuf>, upstream: String, authority: Option<String>) -> anyhow::Result<()> {
    let authority = authority.map(|a| Address::from_str(&a).map_err(|_| anyhow::anyhow!("--authority {a:?} is not a valid pubkey"))).transpose()?;
    let proofs = proofs.unwrap_or_else(|| Paths::for_records(&records).proofs);
    let chain = rpc_chain(&upstream);
    let verdict = verify_line(&chain, &records, &proofs, line, authority.as_ref()).await?;
    println!("{}", render(&verdict));
    match verdict {
        Verdict::Verified { registry, .. } => {
            if authority.is_none() {
                let owner = match registry_authority(&chain, &registry).await {
                    Ok(Some(a)) => a.to_string(),
                    Ok(None) => "unknown (no Registry account)".into(),
                    Err(e) => format!("unknown ({})", escaped(&e.to_string())),
                };
                eprintln!("note: registry {registry} (authority {owner}) was taken from the proofs file; pass --authority <pubkey> to pin it");
            }
            Ok(())
        }
        Verdict::NotVerified(_) => std::process::exit(1),
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
        Cmd::Verify { records, line, proofs, upstream, authority } => verify(records, line, proofs, upstream, authority).await.unwrap_or_else(|e| fatal(e)),
        Cmd::Shadow { upstream, count, slot, out, delay_ms } => aval_svm::shadow::run(&upstream, count, slot, &out, delay_ms).await?,
        Cmd::Serve { config } => {
            let c = Config::load(Some(&config))?;
            let upstream = Upstream::new(&c.upstream_url, &c.commitment, c.upstream_timeout_ms);
            let engine = Engine::new(Cache::new(upstream.clone(), Duration::from_millis(c.cache_ttl_ms)).with_program_ttl(Duration::from_millis(c.program_ttl_ms)), Pool::new(c.pool_size, c.recycle_after));
            let preload: Vec<Address> = DEFAULT_PRELOAD.iter().map(|s| s.to_string()).chain(c.preload_programs.clone())
                .filter_map(|s| Address::from_str(&s).ok()).collect();
            if let Err(e) = engine.cache().get_many(&preload, false).await {
                tracing::warn!("preload failed, programs load on first use: {e}");
            }
            let listener = tokio::net::TcpListener::bind(&c.listen).await?;
            println!("aval-svm on http://{} → upstream {} (pool {}, ttl {} ms)", c.listen, c.upstream_url, c.pool_size, c.cache_ttl_ms);
            axum::serve(listener, router(Arc::new(App { engine, upstream }))).await?;
        }
    }
    Ok(())
}
