/// Runtime configuration read from environment variables at startup.
/// All fields have defaults so the server runs without any configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub port: u16,
    /// Per-engine request timeout in milliseconds
    pub engine_timeout_ms: u64,
    /// How many results to request from each engine
    pub results_per_engine: usize,
    /// How many aggregated results to return to the caller
    pub max_results: usize,
    /// How long to cache aggregated responses in memory, in milliseconds.
    /// `0` disables the response cache.
    pub cache_ttl_ms: u64,
    /// Max concurrent in-flight requests allowed against a single engine.
    pub engine_max_concurrency: usize,
    /// How long an engine that just failed is skipped before retrying, in milliseconds.
    pub engine_cooldown_ms: u64,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            port: env_parse("PORT", 3000),
            engine_timeout_ms: env_parse("ENGINE_TIMEOUT_MS", 8_000),
            results_per_engine: env_parse("RESULTS_PER_ENGINE", 10),
            max_results: env_parse("MAX_RESULTS", 10),
            cache_ttl_ms: env_parse("CACHE_TTL_MS", 60_000),
            engine_max_concurrency: env_parse("ENGINE_MAX_CONCURRENCY", 4),
            engine_cooldown_ms: env_parse("ENGINE_COOLDOWN_MS", 30_000),
        }
    }
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
