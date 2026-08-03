use std::time::Duration;

use reqwest::Client;
use scraper::Html;

use crate::error::EngineError;
use crate::models::SearchResult;

const ENGINE: &str = "yahoo";
const YAHOO_URL: &str = "https://search.yahoo.com/search";
const TIMEOUT_MS: u64 = 8_000;

pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, EngineError> {
    let response = tokio::time::timeout(
        Duration::from_millis(TIMEOUT_MS),
        client
            .get(YAHOO_URL)
            .query(&[("p", query)])
            // Language + safe-search filter cookie (vm=p means safe-search off)
            .header("Cookie", "sB=v=1&vm=p&fl=1&vl=lang_en&pn=10")
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

    parse(&body, max_results)
}

fn parse(html: &str, max_results: usize) -> Result<Vec<SearchResult>, EngineError> {
    let document = Html::parse_document(html);

    let result_sel = sel(ENGINE, "div.algo-sr")?;
    // search.yahoo.com layout: compTitle > a > h3 > span
    let link_sel = sel(ENGINE, "div.compTitle a")?;
    let title_sel = sel(ENGINE, "div.compTitle a h3 span")?;
    let snippet_sel = sel(ENGINE, "div.compText")?;

    let mut results = Vec::new();

    for element in document.select(&result_sel) {
        if results.len() >= max_results {
            break;
        }

        let Some(link_el) = element.select(&link_sel).next() else {
            continue;
        };

        let raw_href = link_el.value().attr("href").unwrap_or("");
        let url = parse_yahoo_url(raw_href);
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
            .map(|el| {
                el.text()
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
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

// Yahoo wraps destination URLs inside a redirect:
//   https://r.search.yahoo.com/_ylt=.../RU=https%3A%2F%2Factual.com/RS=...
// Find the real URL between /RU= and /RS or /RK, then percent-decode it.
fn parse_yahoo_url(href: &str) -> String {
    let start = match href.find("/RU=") {
        Some(pos) => {
            let after = pos + 4; // skip "/RU="
            // The encoded URL starts with http
            match href[after..].find("http") {
                Some(off) => after + off,
                None => return href.to_string(),
            }
        }
        None => return href.to_string(),
    };

    let slice = &href[start..];
    let end = ["/RS=", "/RK="]
        .iter()
        .filter_map(|marker| slice.find(marker))
        .min()
        .unwrap_or(slice.len());

    urlencoding::decode(&slice[..end])
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| slice[..end].to_string())
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
    fn test_parse_yahoo_url_redirect() {
        let href =
            "https://r.search.yahoo.com/_ylt=abc/RU=https%3A%2F%2Fwww.rust-lang.org%2F/RS=xyz";
        assert_eq!(parse_yahoo_url(href), "https://www.rust-lang.org/");
    }

    #[test]
    fn test_parse_yahoo_url_direct() {
        let href = "https://example.com";
        assert_eq!(parse_yahoo_url(href), "https://example.com");
    }

    #[test]
    fn test_parse_yahoo_url_rk_ending() {
        let href = "https://r.search.yahoo.com/_ylt=abc/RU=https%3A%2F%2Fexample.com/RK=2/RS=xyz";
        assert_eq!(parse_yahoo_url(href), "https://example.com");
    }

    #[test]
    fn test_parse_extracts_results() {
        let html = r#"
            <div class="algo-sr">
                <div class="compTitle">
                    <a href="https://r.search.yahoo.com/_ylt=abc/RU=https%3A%2F%2Fexample.com/RS=xyz">
                        <h3><span>Example Site</span></h3>
                    </a>
                </div>
                <div class="compText">An example website for testing.</div>
            </div>
            <div class="algo-sr">
                <div class="compTitle">
                    <a href="https://r.search.yahoo.com/_ylt=abc/RU=https%3A%2F%2Frust-lang.org/RS=xyz">
                        <h3><span>Rust Language</span></h3>
                    </a>
                </div>
                <div class="compText">Systems programming language.</div>
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
        let item = r#"
            <div class="algo-sr">
                <div class="compTitle">
                    <a href="https://r.search.yahoo.com/_ylt=x/RU=https%3A%2F%2Fexample.com/RS=y">
                        <h3><span>Title</span></h3>
                    </a>
                </div>
            </div>
        "#;
        let html = item.repeat(5);
        let results = parse(&html, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_snippet_optional() {
        let html = r#"
            <div class="algo-sr">
                <div class="compTitle">
                    <a href="https://r.search.yahoo.com/_ylt=x/RU=https%3A%2F%2Fexample.com/RS=y">
                        <h3><span>No Snippet</span></h3>
                    </a>
                </div>
            </div>
        "#;
        let results = parse(html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].snippet.is_none());
    }

    #[test]
    fn test_parse_skips_non_http_urls() {
        let html = r#"
            <div class="algo-sr">
                <div class="compTitle">
                    <a href="/relative-link">
                        <h3><span>Relative</span></h3>
                    </a>
                </div>
            </div>
            <div class="algo-sr">
                <div class="compTitle">
                    <a href="https://r.search.yahoo.com/_ylt=x/RU=https%3A%2F%2Fvalid.com/RS=y">
                        <h3><span>Valid</span></h3>
                    </a>
                </div>
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
        let results = search(&client, "rust programming language", 10)
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
            "expected at least one result from Yahoo"
        );
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.url.starts_with("http"));
        }
    }
}
