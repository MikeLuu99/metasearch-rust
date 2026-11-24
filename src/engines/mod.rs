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
    pub client: Arc<Client>,
    pub timeout: Duration,
}

pub struct BraveEngine {
    pub client: Arc<Client>,
    pub timeout: Duration,
}

pub struct StartpageEngine {
    pub client: Arc<Client>,
    pub timeout: Duration,
}

pub struct YahooEngine {
    pub client: Arc<Client>,
    pub timeout: Duration,
}

pub struct DuckDuckGoImagesEngine {
    pub client: Arc<Client>,
    pub timeout: Duration,
}

pub struct BingImagesEngine {
    pub client: Arc<Client>,
    pub timeout: Duration,
}

pub struct GoogleImagesEngine {
    pub client: Arc<Client>,
    pub timeout: Duration,
}

pub struct SogouImagesEngine {
    pub client: Arc<Client>,
    pub timeout: Duration,
}

macro_rules! impl_engine_constructors {
    ($($engine:ident),+ $(,)?) => {
        $(
            impl $engine {
                pub fn new(client: Arc<Client>) -> Self {
                    Self::with_timeout(client, Duration::from_millis(DEFAULT_TIMEOUT_MS))
                }

                pub fn with_timeout(client: Arc<Client>, timeout: Duration) -> Self {
                    Self { client, timeout }
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
            &self.client,
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
            &self.client,
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
            &self.client,
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
            &self.client,
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
            &self.client,
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
            &self.client,
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
            &self.client,
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
            &self.client,
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
pub fn build_http_client() -> anyhow::Result<Client> {
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

    builder.build().map_err(Into::into)
}
