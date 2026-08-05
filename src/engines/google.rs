use std::time::Duration;

use reqwest::{
    Client,
    header::{self, HeaderValue},
};
use scraper::Html;

use crate::error::EngineError;
use crate::models::SearchResult;

const ENGINE: &str = "google";
const GOOGLE_URL: &str = "https://www.google.com/search";

/// Search Google web results.
///
/// Ports SearXNG's `google.py` engine: every organic result is an `<a>` with a
/// `data-ved` attribute and no `class` (SearXNG's `//a[@data-ved and not(@class)]`).
/// The title lives in a `div[style]` inside the anchor, the URL is the `href`
/// (a `/url?q=<encoded>` redirect that must be unwrapped) and the snippet sits
/// in a `div.ilUpNd.H66NU.aSRlid` two levels above the anchor.
pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<SearchResult>, EngineError> {
    let response = tokio::time::timeout(
        timeout,
        client
            .get(GOOGLE_URL)
            .query(&[
                ("q", query),
                ("hl", "en"),
                ("ie", "utf8"),
                ("oe", "utf8"),
                ("filter", "0"),
                ("start", "0"),
            ])
            .header(header::COOKIE, HeaderValue::from_static("CONSENT=YES+"))
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

    detect_google_sorry(&response)?;

    let body = response.text().await.map_err(|e| EngineError::Http {
        engine: ENGINE,
        source: e,
    })?;

    parse(&body, max_results)
}

/// Google answers bot-detection by redirecting to a `sorry`/CAPTCHA page; the
/// redirect is followed by the client, so check where we ended up (mirrors
/// SearXNG's `detect_google_sorry`).
fn detect_google_sorry(response: &reqwest::Response) -> Result<(), EngineError> {
    let url = response.url();
    if url.host_str() == Some("sorry.google.com") || url.path().starts_with("/sorry") {
        return Err(EngineError::Blocked {
            engine: ENGINE,
            reason: "Google served a CAPTCHA / sorry page".to_string(),
        });
    }
    Ok(())
}

fn parse(html: &str, max_results: usize) -> Result<Vec<SearchResult>, EngineError> {
    let document = Html::parse_document(html);

    let result_sel = sel(ENGINE, "a[data-ved]:not([class])")?;
    let title_sel = sel(ENGINE, "div[style]")?;
    let snippet_sel = sel(
        ENGINE,
        "div[class~=\"ilUpNd\"][class~=\"H66NU\"][class~=\"aSRlid\"]",
    )?;

    let mut results = Vec::new();

    for element in document.select(&result_sel) {
        if results.len() >= max_results {
            break;
        }

        let Some(title_el) = element.select(&title_sel).next() else {
            continue; // not one of the common Google result sections
        };

        let title = title_el.text().collect::<String>().trim().to_string();
        if title.is_empty() {
            continue;
        }

        let href = element.value().attr("href").unwrap_or("");
        let url = extract_destination_url(href).unwrap_or_default();
        if url.is_empty() {
            continue;
        }

        // Snippet lives two levels above the anchor, next to the result block.
        let snippet = element
            .parent()
            .and_then(scraper::ElementRef::wrap)
            .and_then(|p| p.parent())
            .and_then(scraper::ElementRef::wrap)
            .and_then(|gp| gp.select(&snippet_sel).next())
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty());

        results.push(SearchResult {
            title,
            url,
            snippet,
            source_engine: ENGINE.to_string(),
        });
    }

    Ok(results)
}

// Google wraps most destinations as redirects: /url?q=<encoded-url>&sa=U&ved=...
// Parse as a full URL and pull out the q parameter value. Direct links are
// returned untouched.
fn extract_destination_url(href: &str) -> Option<String> {
    if !href.starts_with("/url?q=") {
        return Some(href.to_string());
    }

    let full = format!("https://www.google.com{href}");
    let parsed = url::Url::parse(&full).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == "q")
        .map(|(_, v)| v.into_owned())
}

fn sel(engine: &'static str, s: &str) -> Result<scraper::Selector, EngineError> {
    scraper::Selector::parse(s).map_err(|e| EngineError::ParseFailed {
        engine,
        reason: format!("invalid selector '{s}': {e:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_block(title: &str, href: &str, snippet: &str) -> String {
        format!(
            r#"
            <div class="g">
                <div>
                    <a data-ved="0ahUKEwiY" href="{href}">
                        <div style="position:relative">
                            <h3>{title}</h3>
                        </div>
                    </a>
                </div>
                <div class="ilUpNd H66NU aSRlid">
                    <span>{snippet}</span>
                </div>
            </div>"#
        )
    }

    #[test]
    fn test_extract_destination_url() {
        let href = "/url?q=https%3A%2F%2Fwww.rust-lang.org%2F&sa=U&ved=0ahUKEwiY";
        assert_eq!(
            extract_destination_url(href),
            Some("https://www.rust-lang.org/".to_string())
        );
        assert_eq!(
            extract_destination_url("https://example.com"),
            Some("https://example.com".to_string())
        );
    }

    #[test]
    fn test_parse_extracts_results() {
        let html = format!(
            "{}{}",
            result_block(
                "Example Site",
                "/url?q=https%3A%2F%2Fexample.com&sa=U&ved=0ahUKEwiY",
                "An example website for testing.",
            ),
            result_block(
                "Rust",
                "/url?q=https%3A%2F%2Frust-lang.org&sa=U&ved=0ahUKEwiY",
                "Systems programming language.",
            ),
        );

        let results = parse(&html, 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Site");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(
            results[0].snippet.as_deref(),
            Some("An example website for testing.")
        );
        assert_eq!(results[1].url, "https://rust-lang.org");
    }

    #[test]
    fn test_parse_direct_url_without_redirect() {
        let html = result_block("Direct", "https://example.com/direct", "Snippet text");

        let results = parse(&html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/direct");
    }

    #[test]
    fn test_parse_respects_max_results() {
        let block = result_block("T", "/url?q=https%3A%2F%2Fexample.com&sa=U", "S");
        let html = block.repeat(5);

        let results = parse(&html, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_skips_missing_snippet() {
        let html = r#"<div class="g">
                <div>
                    <a data-ved="0ah" href="/url?q=https%3A%2F%2Fexample.com&sa=U">
                        <div style="position:relative"><h3>No Snippet</h3></div>
                    </a>
                </div>
            </div>"#
            .to_string();

        let results = parse(&html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].snippet.is_none());
    }

    #[test]
    fn test_parse_skips_anchors_with_class() {
        // Anchors with a class attribute (e.g. sitelinks, related searches)
        // must not be treated as organic results.
        let html = r#"<div class="g">
                <div>
                    <a class="some-class" data-ved="0ah" href="https://example.com/other">
                        <div style="position:relative"><h3>Not Organic</h3></div>
                    </a>
                </div>
            </div>"#
            .to_string();

        let results = parse(&html, 10).unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_search() {
        let client = crate::engines::build_http_client().unwrap();
        let results = search(
            &client,
            "rust programming language",
            10,
            Duration::from_millis(crate::engines::DEFAULT_TIMEOUT_MS),
        )
        .await
        .unwrap();

        println!("Got {} results:", results.len());
        for r in &results {
            println!("  [{}] {}", r.title, r.url);
            if let Some(s) = &r.snippet {
                println!("    snippet: {}", &s[..s.len().min(80)]);
            }
        }

        assert!(
            !results.is_empty(),
            "expected at least one result from Google"
        );
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.url.starts_with("http"));
        }
    }
}
