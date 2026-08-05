use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
    pub source_engine: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedResult {
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
    pub engines: Vec<String>,
    pub score: f64,
}

/// A single image result from one engine.
///
/// Mirrors SearXNG's `Image` result type: `url` is the page hosting the image,
/// `img_src` is the full-resolution image file, `thumbnail_src` is a smaller
/// preview (falling back to `img_src` when absent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageResult {
    pub title: String,
    /// URL of the page that hosts the image (the referrer page)
    pub url: String,
    /// Direct URL to the full-resolution image file
    pub img_src: String,
    /// URL to a smaller preview image
    pub thumbnail_src: Option<String>,
    /// Name of the site hosting the image
    pub source: Option<String>,
    /// Display resolution, e.g. "1920x1080"
    pub resolution: Option<String>,
    /// Image format, e.g. "png"
    pub img_format: Option<String>,
    pub author: Option<String>,
    pub snippet: Option<String>,
    pub source_engine: String,
}

/// Deduplicated and RRF-ranked image result across engines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedImageResult {
    pub title: String,
    pub url: String,
    pub img_src: String,
    pub thumbnail_src: Option<String>,
    pub source: Option<String>,
    pub resolution: Option<String>,
    pub engines: Vec<String>,
    pub score: f64,
}

/// Query parameters extracted from the HTTP request by the Axum handler.
/// `max_results` is optional and capped by the server config in the handler.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub max_results: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<AggregatedResult>,
    pub engines_queried: Vec<String>,
    pub engines_failed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ImageSearchResponse {
    pub query: String,
    pub results: Vec<AggregatedImageResult>,
    pub engines_queried: Vec<String>,
    pub engines_failed: Vec<String>,
}
