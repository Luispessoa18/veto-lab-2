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
