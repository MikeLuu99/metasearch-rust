pub mod bing_images;
pub mod brave;
pub mod ddg_images;
pub mod duckduckgo;
pub mod google_images;
pub mod startpage;
pub mod yahoo;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::{
    Client,
    header::{self, HeaderMap, HeaderValue},
};

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

// Mimic a real browser as closely as possible to avoid bot-detection rejections.
// These headers are what Chrome 124 sends on a fresh navigation.
pub fn build_http_client() -> anyhow::Result<Client> {
    let mut headers = HeaderMap::new();

    headers.insert(
        header::USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/124.0.0.0 Safari/537.36",
        ),
    );
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    );
    headers.insert(
        header::ACCEPT_LANGUAGE,
        HeaderValue::from_static("en-US,en;q=0.9"),
    );
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("gzip, deflate, br"),
    );
    headers.insert("DNT", HeaderValue::from_static("1"));

    // Sec-Fetch-* headers signal a top-level browser navigation (not an XHR/fetch).
    // Some engines reject requests that omit these.
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
    headers.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));

    let client = Client::builder()
        .default_headers(headers)
        .cookie_store(true)
        .gzip(true)
        .brotli(true)
        // Hard socket-level ceiling; per-query timeouts are applied separately
        .timeout(Duration::from_secs(20))
        .build()?;

    Ok(client)
}
