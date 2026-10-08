use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub listen: String,
    pub upstream_url: String,
    pub commitment: String,
    pub cache_ttl_ms: u64,
    /// Program and ProgramData accounts are refetched after this, so upgrades are seen.
    pub program_ttl_ms: u64,
    pub pool_size: usize,
    /// Rebuild a worker's VM after this many simulations, to bound memory.
    pub recycle_after: u32,
    /// Extra program ids fetched and pinned at boot (e.g. Jupiter, Orca).
    pub preload_programs: Vec<String>,
    pub upstream_timeout_ms: u64,
    /// Second RPC provider; when set, every account read is fetched from both and compared.
    pub upstream_secondary_url: Option<String>,
    /// Most slots the two providers may differ by once a lagging one has been refetched.
    pub quorum_max_slot_gap: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: "127.0.0.1:8899".into(),
            upstream_url: "https://api.devnet.solana.com".into(),
            commitment: "confirmed".into(),
            cache_ttl_ms: 2000,
            program_ttl_ms: 60_000,
            pool_size: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4),
            recycle_after: 5000,
            preload_programs: Vec::new(),
            upstream_timeout_ms: 10_000,
            upstream_secondary_url: None,
            quorum_max_slot_gap: 4,
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> anyhow::Result<Config> {
        let mut config = match path {
            Some(p) if p.exists() => toml::from_str(&std::fs::read_to_string(p)?)?,
            _ => Config::default(),
        };
        if let Ok(v) = std::env::var("AVAL_LISTEN") {
            config.listen = v;
        }
        if let Ok(v) = std::env::var("AVAL_UPSTREAM_URL") {
            config.upstream_url = v;
        }
        if let Ok(v) = std::env::var("AVAL_UPSTREAM_SECONDARY_URL") {
            config.upstream_secondary_url = Some(v);
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_overrides_only_given_fields() {
        let c: Config = toml::from_str("cache_ttl_ms = 500").unwrap();
        assert_eq!(c.cache_ttl_ms, 500);
        assert_eq!(c.listen, "127.0.0.1:8899");
        assert_eq!(c.program_ttl_ms, 60_000);
        assert_eq!((c.upstream_secondary_url, c.quorum_max_slot_gap), (None, 4));
    }

    #[test]
    fn secondary_url_and_gap_come_from_toml() {
        let c: Config = toml::from_str("upstream_secondary_url = \"http://b\"\nquorum_max_slot_gap = 9").unwrap();
        assert_eq!((c.upstream_secondary_url.as_deref(), c.quorum_max_slot_gap), (Some("http://b"), 9));
    }
}
