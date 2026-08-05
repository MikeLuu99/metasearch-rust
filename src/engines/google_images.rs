use std::time::Duration;

use reqwest::{
    Client,
    header::{self, HeaderValue},
};
use serde::Deserialize;

use crate::error::EngineError;
use crate::models::ImageResult;

const ENGINE: &str = "google";
const GOOGLE_URL: &str = "https://www.google.com/search";

/// User-Agent of Google's Go Android app. SearXNG uses it to get ~100 results
/// per page instead of the ~10 a desktop browser UA receives (searxng#1641).
const GOOGLE_UA: &str = "NSTN/3.60.474802233.release Dalvik/2.1.0 (Linux; U; Android 12; US) gzip";

/// Search Google Images.
///
/// Like SearXNG, we use the internal JSON API behind Google's Android app
/// (`async=_fmt:json`). The response is a JSON object (prefixed with `)]}'`
/// to defeat JSON hijacking) whose `ischj.metadata` array carries each image:
/// `referrer_url` (hosting page), `original_image.url` (full image),
/// `thumbnail.url` (preview) plus title, resolution and site.
pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<ImageResult>, EngineError> {
    // The `async` parameter must reach Google raw — percent-encoding it (as
    // reqwest's .query() would) triggers a 403, so the URL is built by hand.
    let url = format!(
        "{GOOGLE_URL}?q={}&tbm=isch&asearch=isch&async=_fmt:json,p:1,ijn:0&hl=en&ie=utf8&oe=utf8",
        urlencoding::encode(query)
    );

    let response = tokio::time::timeout(
        timeout,
        client
            .get(url)
            .header(header::USER_AGENT, HeaderValue::from_static(GOOGLE_UA))
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

    let body = response.text().await.map_err(|e| EngineError::Http {
        engine: ENGINE,
        source: e,
    })?;

    parse_json(&body, max_results)
}

#[derive(Debug, Deserialize)]
struct GoogleImagesResponse {
    ischj: GoogleImagesResults,
}

#[derive(Debug, Deserialize)]
struct GoogleImagesResults {
    #[serde(default)]
    metadata: Vec<GoogleImageItem>,
}

#[derive(Debug, Deserialize)]
struct GoogleImageItem {
    #[serde(default)]
    result: GoogleImageResult,
    #[serde(default)]
    original_image: Option<GoogleImageFile>,
    #[serde(default)]
    thumbnail: Option<GoogleImageFile>,
    #[serde(default)]
    text_in_grid: GoogleTextInGrid,
}

#[derive(Debug, Default, Deserialize)]
struct GoogleImageResult {
    /// Title of the page hosting the image
    #[serde(default)]
    page_title: String,
    /// URL of the page hosting the image
    #[serde(default)]
    referrer_url: String,
    /// Name of the site hosting the image
    #[serde(default)]
    site_title: Option<String>,
    #[serde(default)]
    iptc: GoogleIptc,
}

#[derive(Debug, Default, Deserialize)]
struct GoogleIptc {
    /// Image authors — Google returns a list of creator names
    #[serde(default)]
    creator: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct GoogleImageFile {
    url: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct GoogleTextInGrid {
    #[serde(default)]
    snippet: Option<String>,
}

fn parse_json(body: &str, max_results: usize) -> Result<Vec<ImageResult>, EngineError> {
    // Strip Google's `)]}'` JSON-hijacking prefix before parsing.
    let Some(start) = body.find("{\"ischj\":") else {
        return Err(EngineError::ParseFailed {
            engine: ENGINE,
            reason: "no ischj JSON object found in Google response".to_string(),
        });
    };

    let parsed: GoogleImagesResponse =
        serde_json::from_str(&body[start..]).map_err(|e| EngineError::ParseFailed {
            engine: ENGINE,
            reason: format!("invalid Google images JSON: {e}"),
        })?;

    let mut results = Vec::new();

    for item in parsed.ischj.metadata {
        if results.len() >= max_results {
            break;
        }

        let title = item.result.page_title.trim().to_string();
        let Some(img) = item.original_image else {
            continue;
        };
        if title.is_empty() || item.result.referrer_url.is_empty() || img.url.is_empty() {
            continue;
        }

        let resolution = match (img.width, img.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        };

        results.push(ImageResult {
            title,
            url: item.result.referrer_url,
            img_src: img.url,
            thumbnail_src: item.thumbnail.map(|t| t.url).filter(|u| !u.is_empty()),
            source: item.result.site_title.filter(|s| !s.is_empty()),
            resolution,
            img_format: None,
            author: item
                .result
                .iptc
                .creator
                .map(|creators| creators.join(", "))
                .filter(|c| !c.is_empty()),
            snippet: item.text_in_grid.snippet.filter(|s| !s.is_empty()),
            source_engine: ENGINE.to_string(),
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        page_title: &str,
        referrer_url: &str,
        murl: &str,
        turl: &str,
        site: &str,
        snippet: &str,
    ) -> String {
        format!(
            r#"{{
                "result": {{"page_title": "{page_title}", "referrer_url": "{referrer_url}", "site_title": "{site}",
                           "iptc": {{"creator": ["Photo Author", "Second Author"]}}}},
                "original_image": {{"url": "{murl}", "width": 1920, "height": 1080}},
                "thumbnail": {{"url": "{turl}", "width": 200, "height": 112}},
                "text_in_grid": {{"snippet": "{snippet}"}}
            }}"#
        )
    }

    fn response_body(items: &[String]) -> String {
        format!(
            ")}}]'\n{{\"ischj\":{{\"metadata\":[{}]}}}}",
            items.join(",")
        )
    }

    #[test]
    fn test_parse_extracts_three_urls() {
        let body = response_body(&[item(
            "Rust Logo",
            "https://rust-lang.org/",
            "https://cdn.rust-lang.org/logo.png",
            "https://encrypted-tbn0.gstatic.com/thumb.png",
            "rust-lang.org",
            "The Rust logo.",
        )]);

        let results = parse_json(&body, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust Logo");
        assert_eq!(results[0].url, "https://rust-lang.org/");
        assert_eq!(results[0].img_src, "https://cdn.rust-lang.org/logo.png");
        assert_eq!(
            results[0].thumbnail_src.as_deref(),
            Some("https://encrypted-tbn0.gstatic.com/thumb.png")
        );
        assert_eq!(results[0].source.as_deref(), Some("rust-lang.org"));
        assert_eq!(results[0].resolution.as_deref(), Some("1920x1080"));
        assert_eq!(
            results[0].author.as_deref(),
            Some("Photo Author, Second Author")
        );
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
            "",
        )]);

        let results = parse_json(&body, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].thumbnail_src.is_none());
        assert!(results[0].source.is_none());
        assert!(results[0].snippet.is_none());
        assert_eq!(
            results[0].author.as_deref(),
            Some("Photo Author, Second Author")
        );
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
                    "",
                )
            })
            .collect();
        let body = response_body(&items);

        let results = parse_json(&body, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_skips_item_without_original_image() {
        let body = format!(
            r#")}}]'
            {{"ischj":{{"metadata":[
                {},
                {{"result": {{"page_title": "Good", "referrer_url": "https://example.com"}},
                  "original_image": {{"url": "https://example.com/i.png", "width": 10, "height": 10}},
                  "text_in_grid": {{"snippet": ""}}}}
            ]}}}}"#,
            r#"{"result": {"page_title": "No Image", "referrer_url": "https://example.com"},
                "text_in_grid": {"snippet": ""}}"#
        );

        let results = parse_json(&body, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Good");
    }

    #[test]
    fn test_parse_skips_empty_title() {
        let body = response_body(&[item(
            "  ",
            "https://example.com",
            "https://example.com/i.png",
            "",
            "",
            "",
        )]);

        let results = parse_json(&body, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_no_ischj_returns_error() {
        assert!(parse_json("<html><body>enable js</body></html>", 10).is_err());
        assert!(parse_json(")]}'{\"not\":\"ischj\"}", 10).is_err());
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
            "expected at least one image result from Google"
        );
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.url.starts_with("http"));
            assert!(r.img_src.starts_with("http"));
        }
    }
}
