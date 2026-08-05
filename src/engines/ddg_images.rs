use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use crate::error::EngineError;
use crate::models::ImageResult;

const ENGINE: &str = "duckduckgo";
const DDG_URL: &str = "https://duckduckgo.com/";
const DDG_IMAGES_API: &str = "https://duckduckgo.com/i.js";

/// Search DuckDuckGo images.
///
/// DDG requires a per-query `vqd` token: first fetch the regular images page to
/// extract the token, then query the `i.js` JSON API with it.
pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<ImageResult>, EngineError> {
    let token = fetch_vqd_token(client, query, timeout).await?;

    let response = tokio::time::timeout(
        timeout,
        client
            .get(DDG_IMAGES_API)
            // i.js returns 403 over HTTP/2 — DDG only serves it over HTTP/1.1
            .version(reqwest::Version::HTTP_11)
            .query(&[("l", "us-en"), ("o", "json"), ("q", query), ("vqd", &token)])
            .send(),
    )
    .await
    .map_err(|_| EngineError::Timeout { engine: ENGINE })?
    .map_err(|e| EngineError::Http {
        engine: ENGINE,
        source: e,
    })?;

    if !response.status().is_success() {
        return Err(EngineError::BadStatus {
            engine: ENGINE,
            status: response.status().as_u16(),
        });
    }

    let body = response.text().await.map_err(|e| EngineError::Http {
        engine: ENGINE,
        source: e,
    })?;

    parse_json(&body, max_results)
}

/// Fetch the images search page and extract the `vqd` token it embeds.
async fn fetch_vqd_token(
    client: &Client,
    query: &str,
    timeout: Duration,
) -> Result<String, EngineError> {
    let response = tokio::time::timeout(
        timeout,
        client
            .get(DDG_URL)
            .version(reqwest::Version::HTTP_11)
            .query(&[("q", query), ("iax", "images"), ("ia", "images")])
            .send(),
    )
    .await
    .map_err(|_| EngineError::Timeout { engine: ENGINE })?
    .map_err(|e| EngineError::Http {
        engine: ENGINE,
        source: e,
    })?;

    if !response.status().is_success() {
        return Err(EngineError::BadStatus {
            engine: ENGINE,
            status: response.status().as_u16(),
        });
    }

    let body = response.text().await.map_err(|e| EngineError::Http {
        engine: ENGINE,
        source: e,
    })?;

    extract_vqd(&body).ok_or_else(|| EngineError::ParseFailed {
        engine: ENGINE,
        reason: "could not extract vqd token from DuckDuckGo page".to_string(),
    })
}

/// Pull the `vqd="<token>"` (or `vqd='<token>'`) value out of the page HTML.
fn extract_vqd(html: &str) -> Option<String> {
    const MARKERS: [&str; 2] = ["vqd=\"", "vqd='"];

    for marker in MARKERS {
        let Some(start) = html.find(marker) else {
            continue;
        };
        let rest = &html[start + marker.len()..];

        // The token is closed by the first quote of either kind.
        let end = match (rest.find('"'), rest.find('\'')) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => rest.len(),
        };

        let token = &rest[..end];
        if !token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Some(token.to_string());
        }
    }

    None
}

#[derive(Debug, Deserialize)]
struct DdgImagesResponse {
    #[serde(default)]
    results: Vec<DdgImageItem>,
}

#[derive(Debug, Deserialize)]
struct DdgImageItem {
    title: String,
    /// URL of the page hosting the image
    url: String,
    /// Direct URL of the full-resolution image
    image: String,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

fn parse_json(body: &str, max_results: usize) -> Result<Vec<ImageResult>, EngineError> {
    let parsed: DdgImagesResponse =
        serde_json::from_str(body).map_err(|e| EngineError::ParseFailed {
            engine: ENGINE,
            reason: format!("invalid i.js JSON: {e}"),
        })?;

    let mut results = Vec::new();

    for item in parsed.results {
        if results.len() >= max_results {
            break;
        }

        let title = item.title.trim().to_string();
        if title.is_empty() || item.url.is_empty() || item.image.is_empty() {
            continue;
        }

        let resolution = match (item.width, item.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        };

        results.push(ImageResult {
            title,
            url: item.url,
            img_src: item.image,
            thumbnail_src: item.thumbnail,
            source: item.source,
            resolution,
            img_format: None,
            author: None,
            snippet: None,
            source_engine: ENGINE.to_string(),
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_vqd_double_quotes() {
        let html = r#"<script>const state = {vqd="3-0123456789abcdef"};</script>"#;
        assert_eq!(extract_vqd(html), Some("3-0123456789abcdef".to_string()));
    }

    #[test]
    fn test_extract_vqd_single_quotes() {
        let html = r#"var vqd='7-0987654321fedcba';"#;
        assert_eq!(extract_vqd(html), Some("7-0987654321fedcba".to_string()));
    }

    #[test]
    fn test_extract_vqd_missing() {
        assert_eq!(extract_vqd("<html><body>no token here</body></html>"), None);
    }

    #[test]
    fn test_parse_json_maps_fields() {
        let body = r#"{
            "results": [
                {
                    "title": "Rust logo",
                    "url": "https://rust-lang.org/",
                    "image": "https://rust-lang.org/logo.png",
                    "thumbnail": "https://rust-lang.org/thumb.png",
                    "source": "rust-lang.org",
                    "width": 1920,
                    "height": 1080
                }
            ]
        }"#;

        let results = parse_json(body, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust logo");
        assert_eq!(results[0].url, "https://rust-lang.org/");
        assert_eq!(results[0].img_src, "https://rust-lang.org/logo.png");
        assert_eq!(
            results[0].thumbnail_src.as_deref(),
            Some("https://rust-lang.org/thumb.png")
        );
        assert_eq!(results[0].source.as_deref(), Some("rust-lang.org"));
        assert_eq!(results[0].resolution.as_deref(), Some("1920x1080"));
    }

    #[test]
    fn test_parse_json_optional_fields() {
        let body = r#"{"results": [{"title": "T", "url": "https://example.com", "image": "https://example.com/i.png"}]}"#;
        let results = parse_json(body, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].thumbnail_src.is_none());
        assert!(results[0].resolution.is_none());
        assert!(results[0].source.is_none());
    }

    #[test]
    fn test_parse_json_respects_max_results() {
        let items: Vec<String> = (1..=5)
            .map(|i| {
                format!(
                    r#"{{"title": "T{i}", "url": "https://example{i}.com", "image": "https://example{i}.com/i.png"}}"#
                )
            })
            .collect();
        let body = format!("{{\"results\": [{}]}}", items.join(","));
        let results = parse_json(&body, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_json_skips_empty_title() {
        let body = r#"{"results": [{"title": "  ", "url": "https://example.com", "image": "https://example.com/i.png"}]}"#;
        let results = parse_json(body, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_json_invalid() {
        assert!(parse_json("not json", 10).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_search() {
        let client = crate::engines::build_http_client().unwrap();
        let results = search(
            &client,
            "rust programming language logo",
            10,
            Duration::from_millis(crate::engines::DEFAULT_TIMEOUT_MS),
        )
        .await
        .unwrap();

        println!("Got {} results:", results.len());
        for r in &results {
            println!("  [{}] {}", r.title, r.url);
            println!("      img: {}", r.img_src);
            if let Some(t) = &r.thumbnail_src {
                println!("      thumb: {}", t);
            }
            if let Some(res) = &r.resolution {
                println!("      resolution: {}", res);
            }
        }

        assert!(
            !results.is_empty(),
            "expected at least one image result from DDG"
        );
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.url.starts_with("http"));
            assert!(r.img_src.starts_with("http"));
        }
    }
}
