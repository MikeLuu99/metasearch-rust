use std::time::Duration;

use reqwest::Client;
use scraper::Html;

use crate::error::EngineError;
use crate::models::SearchResult;

const ENGINE: &str = "brave";
const BRAVE_URL: &str = "https://search.brave.com/search";

pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<SearchResult>, EngineError> {
    // Timeout covers send *and* body download (see duckduckgo.rs).
    let fetch = async {
        let response = client
            .get(BRAVE_URL)
            .query(&[("q", query)])
            .send()
            .await
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

fn parse(html: &str, max_results: usize) -> Result<Vec<SearchResult>, EngineError> {
    let document = Html::parse_document(html);

    // data-type="web" is stable across Brave's Svelte rebuilds; the class names
    // on the same element contain hashed suffixes (e.g. svelte-jmfu5f) that change
    // with every frontend deploy, so we anchor on the data attribute instead.
    let result_sel = sel(ENGINE, "div[data-type='web']")?;

    // a.l1 is Brave's consistent link class for the primary result anchor.
    // The href on this element is the direct destination URL (no redirect wrapper).
    let link_sel = sel(ENGINE, "a.l1")?;
    let title_sel = sel(ENGINE, "div.search-snippet-title")?;
    let snippet_sel = sel(ENGINE, "div.generic-snippet")?;

    let mut results = Vec::new();

    for element in document.select(&result_sel) {
        if results.len() >= max_results {
            break;
        }
        let Some(link_el) = element.select(&link_sel).next() else {
            continue;
        };

        let url = link_el.value().attr("href").unwrap_or("").to_string();
        if url.is_empty() || !url.starts_with("http") {
            continue;
        }

        let title = element
            .select(&title_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        let snippet = element
            .select(&snippet_sel)
            .next()
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

fn sel(engine: &'static str, s: &str) -> Result<scraper::Selector, EngineError> {
    scraper::Selector::parse(s).map_err(|e| EngineError::ParseFailed {
        engine,
        reason: format!("invalid selector '{s}': {e:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_extracts_results() {
        let html = r#"
            <div class="snippet" data-type="web">
                <a class="l1" href="https://example.com">
                    <div class="search-snippet-title">Example Site</div>
                </a>
                <div class="generic-snippet">An example website for testing.</div>
            </div>
            <div class="snippet" data-type="web">
                <a class="l1" href="https://rust-lang.org">
                    <div class="search-snippet-title">Rust Language</div>
                </a>
                <div class="generic-snippet">Systems programming language.</div>
            </div>
        "#;

        let results = parse(html, 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Site");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[1].url, "https://rust-lang.org");
        assert!(results[0].snippet.is_some());
    }

    #[test]
    fn test_parse_respects_max_results() {
        let result_html = r#"
            <div class="snippet" data-type="web">
                <a class="l1" href="https://example.com">
                    <div class="search-snippet-title">T</div>
                </a>
            </div>
        "#;
        let html = result_html.repeat(5);
        let results = parse(&html, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_skips_missing_snippet() {
        let html = r#"
            <div class="snippet" data-type="web">
                <a class="l1" href="https://example.com">
                    <div class="search-snippet-title">Title</div>
                </a>
            </div>
        "#;
        let results = parse(html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].snippet.is_none());
    }

    #[test]
    fn test_parse_skips_non_http_urls() {
        let html = r#"
            <div class="snippet" data-type="web">
                <a class="l1" href="/relative">
                    <div class="search-snippet-title">Relative</div>
                </a>
            </div>
            <div class="snippet" data-type="web">
                <a class="l1" href="https://valid.com">
                    <div class="search-snippet-title">Valid</div>
                </a>
            </div>
        "#;
        let results = parse(html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://valid.com");
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
            "expected at least one result from Brave"
        );
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.url.starts_with("http"));
        }
    }
}
