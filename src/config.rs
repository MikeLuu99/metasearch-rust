/// Runtime configuration read from environment variables at startup.
/// All fields have defaults so the server runs without any configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Address to bind the HTTP listener to. Defaults to loopback so the
    /// server is not exposed to the network unless explicitly configured.
    pub host: String,
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
    /// Allowed CORS origins as a comma-separated list (`ALLOWED_ORIGINS`).
    /// Unset or empty keeps the historical fully-permissive CORS behavior;
    /// set it to restrict which web origins may call this API.
    pub allowed_origins: Option<Vec<String>>,
    /// Max requests per minute per client IP (`RATE_LIMIT_PER_MINUTE`).
    /// `0` disables rate limiting.
    pub rate_limit_per_minute: u64,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            host: env_parse("HOST", "127.0.0.1".to_string()),
            port: env_parse("PORT", 3000),
            engine_timeout_ms: env_parse("ENGINE_TIMEOUT_MS", 8_000),
            results_per_engine: env_parse("RESULTS_PER_ENGINE", 10),
            max_results: env_parse("MAX_RESULTS", 10),
            cache_ttl_ms: env_parse("CACHE_TTL_MS", 60_000),
            // Clamp so a zero limit cannot create an unusable semaphore.
            engine_max_concurrency: env_parse("ENGINE_MAX_CONCURRENCY", 4).max(1),
            engine_cooldown_ms: env_parse("ENGINE_COOLDOWN_MS", 30_000),
            allowed_origins: std::env::var("ALLOWED_ORIGINS").ok().and_then(|v| {
                let origins: Vec<String> = v
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                (!origins.is_empty()).then_some(origins)
            }),
            rate_limit_per_minute: env_parse("RATE_LIMIT_PER_MINUTE", 120),
        }
    }
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    match std::env::var(key) {
        Ok(v) => match v.parse() {
            Ok(parsed) => parsed,
            Err(_) => {
                tracing::warn!("invalid value '{v}' for {key}, using default");
                default
            }
        },
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrency_is_clamped_to_at_least_one() {
        unsafe { std::env::set_var("ENGINE_MAX_CONCURRENCY", "0") };
        let config = AppConfig::from_env();
        assert_eq!(config.engine_max_concurrency, 1);
        unsafe { std::env::remove_var("ENGINE_MAX_CONCURRENCY") };
    }
}
