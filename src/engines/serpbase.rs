use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use crate::error::EngineError;
use crate::models::SearchResult;

const ENGINE: &str = "serpbase";
const SERPBASE_URL: &str = "https://api.serpbase.dev/google/search";
const API_KEY_ENV: &str = "SERPBASE_API_KEY";

/// SerpBase is a Google Search Results API. It is opt-in: the engine is
/// only usable when `SERPBASE_API_KEY` is set, and returns an empty result
/// set (not an error) when the key is missing so the aggregator treats it
/// as a silent no-op rather than an engine failure.
pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<SearchResult>, EngineError> {
    let api_key = std::env::var(API_KEY_ENV).unwrap_or_default();
    if api_key.is_empty() {
        tracing::debug!(
            engine = ENGINE,
            "SERPBASE_API_KEY not set - skipping SerpBase engine"
        );
        return Ok(Vec::new());
    }

    let fetch = async {
        // SerpBase is POST-only: JSON body, key in the X-API-Key header.
        let body = serde_json::json!({
            "q": query,
            "hl": "en",
            "gl": "us",
            "page": 1,
            "device": "default",
        });

        let response = client
            .post(SERPBASE_URL)
            .header("X-API-Key", api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| EngineError::Http {
                engine: ENGINE,
                source: e,
            })?;

        // SerpBase returns business errors in a JSON envelope with HTTP 200
        // (status != 0), so a non-2xx status is a transport-level failure.
        if !response.status().is_success() {
            return Err(EngineError::BadStatus {
                engine: ENGINE,
                status: response.status().as_u16(),
            });
        }

        let text = response.text().await.map_err(|e| EngineError::Http {
            engine: ENGINE,
            source: e,
        })?;

        serde_json::from_str::<SerpBaseResponse>(&text).map_err(|e| EngineError::ParseFailed {
            engine: ENGINE,
            reason: format!("invalid SerpBase JSON: {e}"),
        })
    };

    let envelope = tokio::time::timeout(timeout, fetch)
        .await
        .map_err(|_| EngineError::Timeout { engine: ENGINE })??;

    if envelope.status != 0 {
        return Err(EngineError::ParseFailed {
            engine: ENGINE,
            reason: format!(
                "SerpBase business error: status={} error={:?}",
                envelope.status, envelope.error
            ),
        });
    }

    Ok(envelope
        .organic
        .unwrap_or_default()
        .into_iter()
        .take(max_results)
        .map(|r| SearchResult {
            title: r.title,
            url: r.link,
            snippet: r.snippet,
            source_engine: ENGINE.to_string(),
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct SerpBaseResponse {
    #[serde(default)]
    status: i64,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    organic: Option<Vec<OrganicResult>>,
}

#[derive(Debug, Deserialize)]
struct OrganicResult {
    title: String,
    link: String,
    #[serde(default)]
    snippet: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Response envelope copied from the SerpBase docs example for /google/search.
    const SAMPLE_RESPONSE: &str = r#"{
        "status": 0,
        "request_id": "req_123",
        "elapsed_ms": 512,
        "credits_charged": 1,
        "search_type": "search",
        "query": "rust programming",
        "page": 1,
        "organic": [
            {
                "rank": 1,
                "title": "Rust Programming Language",
                "link": "https://www.rust-lang.org/",
                "snippet": "A language empowering everyone to build reliable and efficient software."
            },
            {
                "rank": 2,
                "title": "Learn Rust",
                "link": "https://doc.rust-lang.org/book/",
                "snippet": "The official Rust book."
            }
        ]
    }"#;

    #[test]
    fn test_parse_organic_results() {
        let envelope: SerpBaseResponse = serde_json::from_str(SAMPLE_RESPONSE).unwrap();
        assert_eq!(envelope.status, 0);
        let organic = envelope.organic.unwrap();
        assert_eq!(organic.len(), 2);
        assert_eq!(organic[0].title, "Rust Programming Language");
        assert_eq!(organic[0].link, "https://www.rust-lang.org/");
        assert_eq!(
            organic[1].snippet.as_deref(),
            Some("The official Rust book.")
        );
    }

    #[test]
    fn test_parse_missing_organic_is_empty() {
        let envelope: SerpBaseResponse =
            serde_json::from_str(r#"{"status": 0, "query": "x"}"#).unwrap();
        assert_eq!(envelope.status, 0);
        assert!(envelope.organic.is_none());
    }

    #[test]
    fn test_business_error_envelope_surfaces_status() {
        // SerpBase returns HTTP 200 with status != 0 for business errors.
        let envelope: SerpBaseResponse =
            serde_json::from_str(r#"{"status": 1001, "error": "unauthorized"}"#).unwrap();
        assert_eq!(envelope.status, 1001);
        assert_eq!(envelope.error.as_deref(), Some("unauthorized"));
    }
}
