pub mod bing_images;
pub mod brave;
pub mod ddg_images;
pub mod duckduckgo;
pub mod google_images;
pub mod sogou_images;
pub mod startpage;
pub mod yahoo;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use wreq::Client;

use crate::error::EngineError;
use crate::models::{ImageResult, SearchResult};

// A heap-allocated future that is Send — required for dyn trait + tokio multi-thread.
// Using BoxFuture keeps the trait object-safe; impl Future in trait methods is not.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &'static str;

    fn search<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>>;
}

/// An engine that returns image results.
///
/// Mirrors SearXNG's `categories = ["images"]` engines: results carry the
/// hosting page URL (`url`), the full image (`img_src`) and a preview
/// (`thumbnail_src`) — the three distinct URLs every image engine extracts.
pub trait ImageSearchEngine: Send + Sync {
    fn name(&self) -> &'static str;

    fn search_images<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<ImageResult>, EngineError>>;
}

// Default per-query timeout applied to every engine unless overridden via
// `with_timeout`. Mirrors the old hardcoded ENGINE_TIMEOUT_MS default.
pub const DEFAULT_TIMEOUT_MS: u64 = 8_000;

/// Accept only http(s) URLs. Scraped results must never carry `javascript:`
/// or `data:` URIs through to downstream API consumers.
pub(crate) fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

pub struct DuckDuckGoEngine {
    pub clients: EngineClients,
    pub timeout: Duration,
}

pub struct BraveEngine {
    pub clients: EngineClients,
    pub timeout: Duration,
}

pub struct StartpageEngine {
    pub clients: EngineClients,
    pub timeout: Duration,
}

pub struct YahooEngine {
    pub clients: EngineClients,
    pub timeout: Duration,
}

pub struct DuckDuckGoImagesEngine {
    pub clients: EngineClients,
    pub timeout: Duration,
}

pub struct BingImagesEngine {
    pub clients: EngineClients,
    pub timeout: Duration,
}

pub struct GoogleImagesEngine {
    pub clients: EngineClients,
    pub timeout: Duration,
}

pub struct SogouImagesEngine {
    pub clients: EngineClients,
    pub timeout: Duration,
}

macro_rules! impl_engine_constructors {
    ($($engine:ident),+ $(,)?) => {
        $(
            impl $engine {
                pub fn new(clients: impl Into<EngineClients>) -> Self {
                    Self::with_timeout(clients, Duration::from_millis(DEFAULT_TIMEOUT_MS))
                }

                pub fn with_timeout(clients: impl Into<EngineClients>, timeout: Duration) -> Self {
                    Self {
                        clients: clients.into(),
                        timeout,
                    }
                }
            }
        )+
    };
}

impl_engine_constructors!(
    DuckDuckGoEngine,
    BraveEngine,
    StartpageEngine,
    YahooEngine,
    DuckDuckGoImagesEngine,
    BingImagesEngine,
    GoogleImagesEngine,
    SogouImagesEngine,
);

impl SearchEngine for DuckDuckGoEngine {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
        Box::pin(duckduckgo::search(
            &self.clients,
            query,
            max_results,
            self.timeout,
        ))
    }
}

impl SearchEngine for BraveEngine {
    fn name(&self) -> &'static str {
        "brave"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
        Box::pin(brave::search(
            &self.clients,
            query,
            max_results,
            self.timeout,
        ))
    }
}

impl SearchEngine for StartpageEngine {
    fn name(&self) -> &'static str {
        "startpage"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
        Box::pin(startpage::search(
            &self.clients,
            query,
            max_results,
            self.timeout,
        ))
    }
}

impl SearchEngine for YahooEngine {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
        Box::pin(yahoo::search(
            &self.clients,
            query,
            max_results,
            self.timeout,
        ))
    }
}

impl ImageSearchEngine for DuckDuckGoImagesEngine {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    fn search_images<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<ImageResult>, EngineError>> {
        Box::pin(ddg_images::search(
            &self.clients,
            query,
            max_results,
            self.timeout,
        ))
    }
}

impl ImageSearchEngine for BingImagesEngine {
    fn name(&self) -> &'static str {
        "bing"
    }

    fn search_images<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<ImageResult>, EngineError>> {
        Box::pin(bing_images::search(
            &self.clients,
            query,
            max_results,
            self.timeout,
        ))
    }
}

impl ImageSearchEngine for GoogleImagesEngine {
    fn name(&self) -> &'static str {
        "google"
    }

    fn search_images<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<ImageResult>, EngineError>> {
        Box::pin(google_images::search(
            &self.clients,
            query,
            max_results,
            self.timeout,
        ))
    }
}

impl ImageSearchEngine for SogouImagesEngine {
    fn name(&self) -> &'static str {
        "sogou"
    }

    fn search_images<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<ImageResult>, EngineError>> {
        Box::pin(sogou_images::search(
            &self.clients,
            query,
            max_results,
            self.timeout,
        ))
    }
}

// Impersonate a real browser at the TLS and HTTP/2 layers (JA3/JA4/Akamai
// fingerprints) via wreq's device profiles, not just at the header layer:
// header-only mimicry still fails against fingerprinting engines because the
// rustls handshake of a plain HTTP client identifies it as a non-browser.
// The emulation profile supplies browser-realistic default headers (UA,
// Accept, Sec-Fetch-*), so none are set manually here; per-request headers
// (e.g. Google's Android app UA) still override profile defaults.
//
// ENGINE_EMULATION selects the profile: `firefox-151` (default), `chrome-149`,
// `safari-18-5`, or `none` (plain TLS, the pre-impersonation behavior). Some
// anti-bot walls (e.g. Startpage's Anubis) challenge Chrome and Safari
// fingerprints while passing Firefox's, so the profile is a tunable, not a
// constant — chosen empirically against the live engines.
fn build_client(proxy: Option<&str>) -> anyhow::Result<Client> {
    let profile = std::env::var("ENGINE_EMULATION").unwrap_or_default();
    let builder = Client::builder()
        .cookie_store(true)
        .gzip(true)
        .brotli(true)
        // Hard socket-level ceiling; per-query timeouts are applied separately
        .timeout(Duration::from_secs(20));

    let builder = match profile.as_str() {
        "none" => builder,
        "chrome-149" => builder.emulation(wreq_util::emulate::Profile::Chrome149),
        "safari-18-5" => builder.emulation(wreq_util::emulate::Profile::Safari18_5),
        _ => builder.emulation(wreq_util::emulate::Profile::Firefox151),
    };

    let builder = match proxy {
        Some(url) => builder.proxy(wreq::Proxy::all(url)?),
        None => builder,
    };

    builder.build().map_err(Into::into)
}

/// Build a single direct client (no proxy) — kept for library users.
pub fn build_http_client() -> anyhow::Result<Client> {
    build_client(None)
}

/// Split a comma-separated proxy URL list, dropping empty entries.
pub(crate) fn parse_proxy_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Proxy list for one engine: `ENGINE_<NAME>_PROXIES` (name uppercased)
/// overrides the global `ENGINE_PROXIES`. Empty when neither is set.
pub(crate) fn engine_proxy_env(engine_name: &str) -> Vec<String> {
    let env_name = format!(
        "ENGINE_{}_PROXIES",
        engine_name.to_uppercase().replace(['-', ' '], "_")
    );
    let raw = std::env::var(&env_name)
        .or_else(|_| std::env::var("ENGINE_PROXIES"))
        .unwrap_or_default();
    parse_proxy_list(&raw)
}

/// A cycling pool of HTTP clients for one engine, mirroring SearXNG's
/// per-engine `Network`.
///
/// Built from `ENGINE_<NAME>_PROXIES` (or the global `ENGINE_PROXIES`) — a
/// comma-separated proxy URL list. Each [`EngineClients::get`] hands out the
/// next client, so requests spread across proxies instead of concentrating
/// blocks on one identity. With no proxies configured the pool is a single
/// direct client (the previous behavior). `retries` adds extra attempts
/// cycling through the pool after a retryable failure (5xx/403 or a
/// transport error); 429 is never retried — retrying an actively throttling
/// upstream prolongs the block.
#[derive(Clone)]
pub struct EngineClients {
    pub(crate) clients: Vec<Arc<Client>>,
    pub(crate) next: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) retries: usize,
}

impl EngineClients {
    /// One direct client, no proxies, no retries.
    pub fn direct() -> anyhow::Result<Self> {
        Ok(Self {
            clients: vec![Arc::new(build_client(None)?)],
            next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            retries: 0,
        })
    }

    /// Number of clients in the pool.
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Whether the pool is empty (cannot happen through the constructors).
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Number of attempts a search should make: `1 + retries`, capped so a
    /// single-client pool never retries through the same identity.
    pub(crate) fn attempts(&self) -> usize {
        if self.retries == 0 || self.clients.len() <= 1 {
            1
        } else {
            1 + self.retries
        }
    }

    /// Next client in the pool, round-robin.
    pub(crate) fn get(&self) -> Arc<Client> {
        let i = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.clients.len();
        Arc::clone(&self.clients[i])
    }
}

impl From<Arc<Client>> for EngineClients {
    fn from(client: Arc<Client>) -> Self {
        Self {
            clients: vec![client],
            next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            retries: 0,
        }
    }
}

/// Build a pool for one engine from its proxy env config.
///
/// `retries` is the number of extra attempts after a retryable failure.
pub fn build_client_pool(engine_name: &str, retries: usize) -> anyhow::Result<EngineClients> {
    let proxies = engine_proxy_env(engine_name);
    let mut clients = Vec::new();
    if proxies.is_empty() {
        clients.push(Arc::new(build_client(None)?));
    } else {
        for url in &proxies {
            clients.push(Arc::new(build_client(Some(url))?));
        }
    }
    Ok(EngineClients {
        clients,
        next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        retries,
    })
}

/// Fetch a response body with per-attempt timeout, proxy cycling and retries.
///
/// Each attempt uses the next client in the pool. Retryable failures (5xx,
/// 403, transport errors, attempt timeouts) consume another attempt through a
/// different identity; the last attempt's error is returned.
pub(crate) async fn fetch_text(
    engine: &'static str,
    clients: &EngineClients,
    timeout: Duration,
    build: impl Fn(&Client) -> wreq::RequestBuilder,
) -> Result<String, EngineError> {
    let attempts = clients.attempts();
    let mut last_err: Option<EngineError> = None;

    for attempt in 0..attempts {
        let client = clients.get();
        // The timeout must cover the whole exchange, including the body
        // download; otherwise a slow-drip server stalls the worker far past
        // the configured budget.
        let fetch = async {
            let response = build(&client)
                .send()
                .await
                .map_err(|e| EngineError::Http { engine, source: e })?;

            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|e| EngineError::Http { engine, source: e })?;

            if !status.is_success() {
                return Err(EngineError::BadStatus {
                    engine,
                    status: status.as_u16(),
                });
            }
            Ok(body)
        };

        match tokio::time::timeout(timeout, fetch).await {
            Ok(Ok(body)) => return Ok(body),
            Ok(Err(e @ EngineError::BadStatus { status, .. })) => {
                // 429 must never be retried: hammering a throttling upstream
                // prolongs the block. 403 and 5xx may succeed via another
                // identity.
                let retryable = (status == 403 || status >= 500) && attempt + 1 < attempts;
                if !retryable {
                    return Err(e);
                }
                last_err = Some(e);
            }
            Ok(Err(e)) => {
                // Transport error: retry through the next identity.
                if attempt + 1 >= attempts {
                    return Err(e);
                }
                last_err = Some(e);
            }
            Err(_) => {
                let e = EngineError::Timeout { engine };
                if attempt + 1 >= attempts {
                    return Err(e);
                }
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or(EngineError::Timeout { engine }))
}

#[cfg(test)]
mod pool_tests {
    use super::*;

    #[test]
    fn parse_proxy_list_splits_trims_and_drops_empty() {
        assert_eq!(
            parse_proxy_list(" http://a:1 , http://b:2 ,,"),
            vec!["http://a:1", "http://b:2"]
        );
        assert!(parse_proxy_list("").is_empty());
        assert!(parse_proxy_list("  ,  ").is_empty());
    }

    #[test]
    fn engine_proxy_env_prefers_engine_specific_over_global() {
        unsafe {
            std::env::set_var("ENGINE_POOL_TEST_PROXIES", "http://engine-specific:1");
            std::env::set_var("ENGINE_PROXIES", "http://global:1");
        }
        assert_eq!(
            engine_proxy_env("pool-test"),
            vec!["http://engine-specific:1"]
        );
        unsafe {
            std::env::remove_var("ENGINE_POOL_TEST_PROXIES");
        }
        assert_eq!(engine_proxy_env("pool-test"), vec!["http://global:1"]);
        unsafe {
            std::env::remove_var("ENGINE_PROXIES");
        }
        assert!(engine_proxy_env("pool-test").is_empty());
    }

    #[tokio::test]
    async fn pool_cycles_round_robin_across_clients() {
        let pool = build_client_pool("cycle-test", 0).unwrap();
        // No proxies configured: single direct client.
        assert_eq!(pool.len(), 1);

        unsafe {
            std::env::set_var(
                "ENGINE_POOL_CYCLE_PROXIES",
                "http://a:1,http://b:2,http://c:3",
            );
        }
        let pool = build_client_pool("pool-cycle", 0).unwrap();
        unsafe {
            std::env::remove_var("ENGINE_POOL_CYCLE_PROXIES");
        }
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.attempts(), 1); // retries=0: a single attempt per search

        // get() hands out clients round-robin: 0, 1, 2, 0, ...
        let first = pool.get();
        let second = pool.get();
        let third = pool.get();
        let fourth = pool.get();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&second, &third));
        assert!(Arc::ptr_eq(&first, &fourth));
    }
}
