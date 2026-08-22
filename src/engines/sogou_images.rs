use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use super::is_http_url;
use crate::error::EngineError;
use crate::models::ImageResult;

const ENGINE: &str = "sogou";
const SOGOU_URL: &str = "https://pic.sogou.com/pics";

/// Search Sogou Images.
///
/// Mirrors SearXNG's `sogou_images.py`: the result page embeds the image list
/// as `window.__INITIAL_STATE__ = {...};`. Each entry under `searchList`
/// carries `url` (hosting page), `picUrl` (used for both the full image and
/// the thumbnail), `title`, `content_major` (snippet) and `ch_site_name`
/// (source site).
pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<ImageResult>, EngineError> {
    let url = format!("{SOGOU_URL}?query={}&start=0", urlencoding::encode(query));

    // Timeout covers send *and* body download (see duckduckgo.rs).
    let fetch = async {
        let response = client.get(url).send().await.map_err(|e| EngineError::Http {
            engine: ENGINE,
            source: e,
        })?;

        if !response.status().is_success() {
            return Err(EngineError::BadStatus {
                engine: ENGINE,
                status: response.status().as_u16(),
            });
        }

        response.text().await.map_err(|e| EngineError::Http {
            engine: ENGINE,
            source: e,
        })
    };

    let body = tokio::time::timeout(timeout, fetch)
        .await
        .map_err(|_| EngineError::Timeout { engine: ENGINE })??;

    parse(&body, max_results)
}

#[derive(Debug, Default, Deserialize)]
struct SogouInitialState {
    #[serde(default, rename = "searchList")]
    search_list: SogouSearchList,
}

#[derive(Debug, Default, Deserialize)]
struct SogouSearchList {
    #[serde(default, rename = "searchList")]
    search_list: Vec<SogouImageItem>,
}

#[derive(Debug, Default, Deserialize)]
struct SogouImageItem {
    #[serde(default)]
    url: String,
    #[serde(default, rename = "picUrl")]
    pic_url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content_major: Option<String>,
    #[serde(default)]
    ch_site_name: Option<String>,
}

fn parse(body: &str, max_results: usize) -> Result<Vec<ImageResult>, EngineError> {
    // Sogou embeds the results as a JSON blob assigned to a JS variable. Locate
    // the first `{` after the marker and scan for its balanced closing brace
    // (skipping quoted strings), which is more robust than SearXNG's
    // non-greedy `({.*?});` regex when the object itself ends in `}};`.
    // A missing state blob almost always means Sogou served a CAPTCHA or a
    // JS-challenge page. Returning an error (rather than an empty success)
    // ensures the failure is recorded and the engine enters cooldown instead
    // of caching an authoritative-looking "0 results" response.
    let Some(state_json) = extract_initial_state(body) else {
        return Err(EngineError::ParseFailed {
            engine: ENGINE,
            reason: "window.__INITIAL_STATE__ JSON not found in response".to_string(),
        });
    };

    let state: SogouInitialState =
        serde_json::from_str(state_json).map_err(|e| EngineError::ParseFailed {
            engine: ENGINE,
            reason: format!("invalid Sogou images JSON: {e}"),
        })?;

    let mut results = Vec::new();

    for item in state.search_list.search_list {
        if results.len() >= max_results {
            break;
        }

        let title = item.title.trim().to_string();
        let pic = item.pic_url.trim().to_string();
        if title.is_empty() || !is_http_url(&item.url) || !is_http_url(&pic) {
            tracing::debug!(engine = ENGINE, "skipping image with missing/invalid urls");
            continue;
        }

        results.push(ImageResult {
            title,
            url: item.url,
            img_src: pic.clone(),
            thumbnail_src: Some(pic),
            source: item.ch_site_name.filter(|s| !s.trim().is_empty()),
            resolution: None,
            img_format: None,
            author: None,
            snippet: item.content_major.filter(|s| !s.trim().is_empty()),
            source_engine: ENGINE.to_string(),
        });
    }

    Ok(results)
}

/// Extract the balanced JSON object starting at the first `{` after the
/// `window.__INITIAL_STATE__` marker, ignoring braces inside string literals.
fn extract_initial_state(body: &str) -> Option<&str> {
    let marker = "window.__INITIAL_STATE__";
    let idx = body.find(marker)?;
    let open = body[idx..].find('{')?;
    let start = idx + open;

    let mut depth = 0u32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in body.as_bytes()[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&body[start..start + offset + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, url: &str, pic_url: &str, snippet: &str, site: &str) -> String {
        format!(
            r#"{{
                "url": "{url}",
                "picUrl": "{pic_url}",
                "title": "{title}",
                "content_major": "{snippet}",
                "ch_site_name": "{site}"
            }}"#
        )
    }

    fn response_body(items: &[String]) -> String {
        format!(
            r#"<html><script>window.__INITIAL_STATE__ = {{"searchList": {{"searchList": [{}]}}}};</script></html>"#,
            items.join(",")
        )
    }

    #[test]
    fn test_parse_extracts_three_urls() {
        let body = response_body(&[item(
            "Rust Logo",
            "https://rust-lang.org/",
            "https://pic.sogou.com/i.png",
            "The Rust logo.",
            "rust-lang.org",
        )]);

        let results = parse(&body, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust Logo");
        assert_eq!(results[0].url, "https://rust-lang.org/");
        assert_eq!(results[0].img_src, "https://pic.sogou.com/i.png");
        assert_eq!(
            results[0].thumbnail_src.as_deref(),
            Some("https://pic.sogou.com/i.png")
        );
        assert_eq!(results[0].source.as_deref(), Some("rust-lang.org"));
        assert_eq!(results[0].snippet.as_deref(), Some("The Rust logo."));
    }

    #[test]
    fn test_parse_optional_fields() {
        let body = response_body(&[item(
            "T",
            "https://example.com",
            "https://example.com/i.png",
            "",
            "",
        )]);

        let results = parse(&body, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].source.is_none());
        assert!(results[0].snippet.is_none());
    }

    #[test]
    fn test_parse_respects_max_results() {
        let items: Vec<String> = (1..=5)
            .map(|i| {
                item(
                    &format!("T{i}"),
                    &format!("https://example{i}.com"),
                    &format!("https://example{i}.com/i.png"),
                    "",
                    "",
                )
            })
            .collect();
        let body = response_body(&items);

        let results = parse(&body, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_skips_empty_title() {
        let body = response_body(&[item(
            "  ",
            "https://example.com",
            "https://example.com/i.png",
            "",
            "",
        )]);

        let results = parse(&body, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_missing_initial_state_returns_error() {
        // A missing state blob usually means a CAPTCHA/JS-challenge page; it
        // must be an error so cooldowns trigger and empty results are not
        // cached as successful responses.
        assert!(parse("<html><body>enable js</body></html>", 10).is_err());
        assert!(parse("window.__INITIAL_STATE__", 10).is_err());
    }

    #[test]
    fn test_parse_invalid_json_returns_error() {
        let body = r#"<script>window.__INITIAL_STATE__ = {"searchList": {"searchList": [invalid}};</script>"#;
        assert!(parse(body, 10).is_err());
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
            if let Some(s) = &r.source {
                println!("      source: {}", s);
            }
        }

        assert!(
            !results.is_empty(),
            "expected at least one image result from Sogou"
        );
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.url.starts_with("http"));
            assert!(r.img_src.starts_with("http"));
        }
    }
}
