use std::collections::HashSet;
use std::time::Duration;

use reqwest::Client;
use scraper::Html;

use crate::error::EngineError;
use crate::models::SearchResult;

const ENGINE: &str = "startpage";
const STARTPAGE_URL: &str = "https://www.startpage.com/search";

// Startpage yields roughly 10-17 organic results per page; fetch more pages
// until max_results is satisfied. Bounded so latency can't blow up.
const MAX_PAGES: usize = 5;

/// Markers of anti-bot challenge pages (Startpage fronts search with the
/// Anubis proof-of-work middleware). A challenged response contains no
/// results, and treating it as a successful empty result would cache a
/// bogus "0 results" answer — it must surface as an engine error instead.
const CHALLENGE_MARKERS: &[&str] = &[
    "anubis_challenge",
    "anubis_version",
    "/cdn-cgi/challenge-platform/",
    "verifying you are human",
    "captcha-delivery.com",
];

pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<SearchResult>, EngineError> {
    search_url(client, STARTPAGE_URL, query, max_results, timeout).await
}

/// Pagination-aware fetch loop against a given Startpage base URL
/// (`base` is injectable so tests can point at a mock server).
async fn search_url(
    client: &Client,
    base: &str,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<SearchResult>, EngineError> {
    let mut results: Vec<SearchResult> = Vec::new();
    let mut seen_urls: HashSet<String> = HashSet::new();

    for page in 1..=MAX_PAGES {
        if results.len() >= max_results {
            break;
        }

        let body = fetch_page(client, base, query, page, timeout).await?;

        if let Some(marker) = CHALLENGE_MARKERS.iter().find(|m| body.contains(**m)) {
            // A challenge on page 1 means we have nothing usable — fail so
            // cooldown triggers instead of caching an empty success. On
            // later pages keep whatever earlier pages produced.
            if results.is_empty() {
                return Err(EngineError::ParseFailed {
                    engine: ENGINE,
                    reason: format!("bot challenge page detected (marker '{marker}')"),
                });
            }
            tracing::warn!(
                engine = ENGINE,
                page,
                marker,
                "bot challenge mid-pagination; keeping partial results"
            );
            break;
        }

        let before = results.len();
        for result in parse(&body, max_results - results.len())? {
            if seen_urls.insert(result.url.clone()) {
                results.push(result);
            }
        }

        // A page that adds nothing new means we've exhausted the result set.
        if results.len() == before {
            break;
        }
    }

    Ok(results)
}

async fn fetch_page(
    client: &Client,
    base: &str,
    query: &str,
    page: usize,
    timeout: Duration,
) -> Result<String, EngineError> {
    // Timeout covers send *and* body download (see duckduckgo.rs).
    let fetch = async {
        // Page 1 keeps the plain ?q= URL; later pages add the pagination
        // parameter (verified to return distinct result sets).
        let params: Vec<(&str, String)> = if page == 1 {
            vec![("q", query.to_string())]
        } else {
            vec![("q", query.to_string()), ("page", page.to_string())]
        };

        let response =
            client
                .get(base)
                .query(&params)
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

    tokio::time::timeout(timeout, fetch)
        .await
        .map_err(|_| EngineError::Timeout { engine: ENGINE })?
}

fn parse(html: &str, max_results: usize) -> Result<Vec<SearchResult>, EngineError> {
    let document = Html::parse_document(html);

    // Startpage uses Emotion CSS-in-JS — class names have unstable hashes appended
    // (e.g. "result css-o7i03b"). The "result" class is the stable anchor;
    // we select all divs whose class list contains exactly "result" as one token.
    let result_sel = sel(ENGINE, "div.result")?;

    // a.result-title holds both the href (destination URL) and wraps the h2 title.
    // Startpage links directly — no redirect wrapper.
    let link_sel = sel(ENGINE, "a.result-title")?;
    let title_sel = sel(ENGINE, "h2.wgl-title")?;
    let snippet_sel = sel(ENGINE, "p.description")?;

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
    use crate::engines::build_http_client;

    #[test]
    fn test_parse_extracts_results() {
        let html = r#"
            <div class="result css-o7i03b">
                <a class="result-title result-link css-abc" href="https://rust-lang.org/">
                    <h2 class="wgl-title css-xyz">Rust Programming Language</h2>
                </a>
                <p class="description css-def">A fast, memory-safe language.</p>
            </div>
            <div class="result css-o7i03b">
                <a class="result-title result-link css-abc" href="https://en.wikipedia.org/wiki/Rust">
                    <h2 class="wgl-title css-xyz">Rust - Wikipedia</h2>
                </a>
                <p class="description css-def">Rust is a general-purpose programming language.</p>
            </div>
        "#;

        let results = parse(html, 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Programming Language");
        assert_eq!(results[0].url, "https://rust-lang.org/");
        assert_eq!(results[1].url, "https://en.wikipedia.org/wiki/Rust");
        assert!(results[0].snippet.is_some());
    }

    #[test]
    fn test_parse_respects_max_results() {
        let block = r#"
            <div class="result css-o7i03b">
                <a class="result-title" href="https://example.com">
                    <h2 class="wgl-title">Title</h2>
                </a>
            </div>
        "#;
        let html = block.repeat(5);
        let results = parse(&html, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_skips_missing_snippet() {
        let html = r#"
            <div class="result css-o7i03b">
                <a class="result-title" href="https://example.com">
                    <h2 class="wgl-title">Title</h2>
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
            <div class="result css-o7i03b">
                <a class="result-title" href="/relative/path">
                    <h2 class="wgl-title">Relative</h2>
                </a>
            </div>
            <div class="result css-o7i03b">
                <a class="result-title" href="https://valid.com">
                    <h2 class="wgl-title">Valid</h2>
                </a>
            </div>
        "#;
        let results = parse(html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://valid.com");
    }

    fn result_block(url: &str, title: &str) -> String {
        format!(
            r#"<div class="result css-x"><a class="result-title" href="{url}"><h2 class="wgl-title">{title}</h2></a></div>"#
        )
    }

    fn page_body(count: usize, offset: usize) -> String {
        (0..count)
            .map(|i| {
                result_block(
                    &format!("https://example.com/{offset}/{i}"),
                    &format!("T{offset}-{i}"),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn test_challenge_page_returns_error_not_empty() {
        // Mirror of a real Anubis proof-of-work wall: 200 OK, no results.
        let mock = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"<html><head><script id="anubis_challenge" type="application/json">{"rules":{}}</script></head><body></body></html>"#,
            ))
            .mount(&mock)
            .await;

        let client = build_http_client().unwrap();
        let err = search_url(
            &client,
            &mock.uri(),
            "rust",
            10,
            Duration::from_millis(crate::engines::DEFAULT_TIMEOUT_MS),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, EngineError::ParseFailed { reason, .. } if reason.contains("bot challenge")),
            "expected challenge ParseFailed, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_pagination_fetches_more_pages_until_max_results() {
        let mock = wiremock::MockServer::start().await;

        // Page 1 (no page param): 10 results. Page 2: 5 more. Page 3+: empty.
        let page1 = page_body(10, 1);
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::query_param_is_missing("page"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(page1))
            .mount(&mock)
            .await;

        let page2 = page_body(5, 2);
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::query_param("page", "2"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(page2))
            .mount(&mock)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::query_param("page", "3"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("<html></html>"))
            .mount(&mock)
            .await;

        let client = build_http_client().unwrap();
        let results = search_url(
            &client,
            &mock.uri(),
            "rust",
            15,
            Duration::from_millis(crate::engines::DEFAULT_TIMEOUT_MS),
        )
        .await
        .unwrap();

        // 10 from page 1 + 5 from page 2 = 15; page 3 is never needed.
        assert_eq!(results.len(), 15);
        assert!(results.iter().all(|r| r.url.starts_with("http")));
    }

    #[tokio::test]
    async fn test_pagination_stops_at_empty_page() {
        let mock = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::query_param_is_missing("page"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(page_body(4, 1)))
            .expect(1)
            .mount(&mock)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::query_param("page", "2"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("<html></html>"))
            .expect(1)
            .mount(&mock)
            .await;

        let client = build_http_client().unwrap();
        let results = search_url(
            &client,
            &mock.uri(),
            "rust",
            50,
            Duration::from_millis(crate::engines::DEFAULT_TIMEOUT_MS),
        )
        .await
        .unwrap();

        // Only the 4 real results — no infinite loop on empty pages.
        assert_eq!(results.len(), 4);
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
            "expected at least one result from Startpage"
        );
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.url.starts_with("http"));
        }
    }
}
