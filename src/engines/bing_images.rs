use std::time::Duration;

use reqwest::Client;
use scraper::Html;
use serde::Deserialize;

use crate::error::EngineError;
use crate::models::ImageResult;

const ENGINE: &str = "bing";
const BING_IMAGES_URL: &str = "https://www.bing.com/images/async";

/// Search Bing Images.
///
/// Like SearXNG, we hit the lightweight `/images/async` endpoint instead of
/// the full `/images/search` page: it is cheaper, returns relevant results
/// without bot-detection decoys, and each result is a `<li>` inside the
/// `dgControl_list` containing an `<a class="iusc">` whose `m` attribute
/// holds an inline JSON blob with `purl` (hosting page), `murl` (full
/// image) and `turl` (thumbnail).
pub async fn search(
    client: &Client,
    query: &str,
    max_results: usize,
    timeout: Duration,
) -> Result<Vec<ImageResult>, EngineError> {
    let response = tokio::time::timeout(
        timeout,
        client
            .get(BING_IMAGES_URL)
            .query(&[
                ("q", query),
                ("async", "1"),
                ("first", "1"),
                ("count", "35"),
            ])
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

/// JSON blob found in each result's `m` attribute.
#[derive(Debug, Deserialize)]
struct BingMetadata {
    /// URL of the page hosting the image
    #[serde(default)]
    purl: String,
    /// Direct URL of the full-resolution image
    #[serde(default)]
    murl: String,
    /// Thumbnail URL of the preview image
    #[serde(default)]
    turl: String,
    /// Snippet/content describing the image
    #[serde(default)]
    desc: Option<String>,
    /// Title from the JSON blob (fallback when the DOM has no title)
    #[serde(default)]
    t: Option<String>,
}

fn parse(html: &str, max_results: usize) -> Result<Vec<ImageResult>, EngineError> {
    let document = Html::parse_document(html);

    let result_sel = sel(ENGINE, "ul.dgControl_list li")?;
    let m_sel = sel(ENGINE, "a.iusc")?;
    let title_sel = sel(ENGINE, "div.infnmpt a")?;
    let resolution_sel = sel(ENGINE, "span.nowrap")?;
    let source_sel = sel(ENGINE, "div.lnkw a")?;

    let mut results = Vec::new();

    for result in document.select(&result_sel) {
        if results.len() >= max_results {
            break;
        }

        let Some(m_el) = result.select(&m_sel).next() else {
            continue; // non-result entries (related searches, etc.)
        };

        let Some(m_attr) = m_el.value().attr("m") else {
            continue;
        };

        let metadata: BingMetadata = match serde_json::from_str(m_attr) {
            Ok(m) => m,
            Err(_) => continue, // ads and other non-result entries lack valid m blobs
        };

        if metadata.purl.is_empty() || metadata.murl.is_empty() {
            continue;
        }

        let title = result
            .select(&title_sel)
            .next()
            .and_then(|el| {
                el.value()
                    .attr("title")
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        let text = el.text().collect::<String>().trim().to_string();
                        (!text.is_empty()).then_some(text)
                    })
            })
            .or(metadata.t)
            .unwrap_or_default()
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }

        let source = result
            .select(&source_sel)
            .next()
            .and_then(|el| el.value().attr("title").map(str::to_string))
            .filter(|s| !s.is_empty());

        let (resolution, img_format) = result
            .select(&resolution_sel)
            .next()
            .map(|el| {
                let text = el.text().collect::<String>().trim().to_string();
                let mut parts = text.split(" · ");
                let resolution = parts
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(normalize_resolution);
                let format = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
                (resolution, format)
            })
            .unwrap_or((None, None));

        results.push(ImageResult {
            title,
            url: metadata.purl,
            img_src: metadata.murl,
            thumbnail_src: Some(metadata.turl).filter(|t| !t.is_empty()),
            source,
            resolution,
            img_format,
            author: None,
            snippet: metadata.desc,
            source_engine: ENGINE.to_string(),
        });
    }

    Ok(results)
}

/// Bing reports dimensions as "3840×2160"; normalize to "3840x2160".
fn normalize_resolution(resolution: &str) -> String {
    resolution.replace('×', "x")
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

    /// Build one result `<li>` mirroring the real `/images/async` layout:
    /// `a.iusc` with the `m` blob, `span.nowrap` for the resolution and
    /// `div.infnmpt a` for the title.
    fn result_li(m: &str, title: &str, nowrap: &str, source: &str) -> String {
        // Real Bing HTML escapes the JSON's quotes inside the m attribute
        let m = m.replace('"', "&quot;");
        format!(
            r#"<li data-idx="1">
                <div class="iuscp isv smallheight">
                    <div class="imgpt">
                        <a class="iusc" m="{m}"><div class="img_cont hoff"><img src="https://th.bing.com/thumb.jpg"/></div></a>
                        <div class="img_info hon">
                            <span class="nowrap">{nowrap}</span>
                            <div class="lnkw"><a title="{source}" href="https://example.com/page">source</a></div>
                        </div>
                    </div>
                    <div class="infnmpt">
                        <div class="infpd hoff">
                            <ul class="b_dataList"><li><a title="{title}" href="/images/search?view=detailV2">{title}</a></li></ul>
                        </div>
                    </div>
                </div>
            </li>"#
        )
    }

    fn search_page(items: &[String]) -> String {
        format!(
            r#"<ul class="dgControl_list" data-infullrow="1">{}</ul>"#,
            items.join("\n")
        )
    }

    #[test]
    fn test_parse_extracts_three_urls() {
        let m = r#"{"purl":"https://example.com/page","murl":"https://cdn.example.com/img.jpg","turl":"https://cdn.example.com/thumb.jpg","desc":"A description","t":"JSON title"}"#;
        let html = search_page(&[result_li(m, "DOM Title", "3840&#215;2160", "example.com")]);

        let results = parse(&html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "DOM Title");
        assert_eq!(results[0].url, "https://example.com/page");
        assert_eq!(results[0].img_src, "https://cdn.example.com/img.jpg");
        assert_eq!(
            results[0].thumbnail_src.as_deref(),
            Some("https://cdn.example.com/thumb.jpg")
        );
        assert_eq!(results[0].resolution.as_deref(), Some("3840x2160"));
        assert_eq!(results[0].source.as_deref(), Some("example.com"));
        assert_eq!(results[0].snippet.as_deref(), Some("A description"));
    }

    #[test]
    fn test_parse_title_from_infnmpt_text_falls_back_to_json() {
        let m = r#"{"purl":"https://example.com/page","murl":"https://cdn.example.com/img.jpg","t":"JSON Title"}"#;
        let html = search_page(&[result_li(m, "", "800&#215;600", "example.com")]);

        let results = parse(&html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "JSON Title");
    }

    #[test]
    fn test_parse_skips_entries_without_murl() {
        let m = r#"{"purl":"https://example.com/page","turl":"https://cdn.example.com/thumb.jpg"}"#;
        let html = search_page(&[result_li(m, "No Image", "", "")]);

        let results = parse(&html, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_skips_invalid_m_json() {
        let html = search_page(&[result_li("not-json", "Broken", "", "")]);

        let results = parse(&html, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_respects_max_results() {
        let m = r#"{"purl":"https://example.com/page","murl":"https://cdn.example.com/img.jpg"}"#;
        let item = result_li(m, "Title", "", "");
        let html = search_page(&vec![item.clone(); 5]);

        let results = parse(&html, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_optional_thumbnail_and_resolution() {
        let m = r#"{"purl":"https://example.com/page","murl":"https://cdn.example.com/img.jpg"}"#;
        let html = search_page(&[result_li(m, "No Thumb", "", "")]);

        let results = parse(&html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].thumbnail_src.is_none());
        assert!(results[0].resolution.is_none());
    }

    #[test]
    fn test_parse_skips_entries_without_iusc() {
        let html = search_page(&[
            result_li(
                r#"{"purl":"https://example.com/page","murl":"https://cdn.example.com/img.jpg"}"#,
                "Good",
                "",
                "",
            ),
            r#"<li class="b_algo"><h2>Not an image result</h2></li>"#.to_string(),
        ]);

        let results = parse(&html, 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_parse_split_format_from_resolution() {
        let m = r#"{"purl":"https://example.com/page","murl":"https://cdn.example.com/img.jpg"}"#;
        let html = search_page(&[result_li(
            m,
            "With Format",
            "1024&#215;768 · JPEG",
            "example.com",
        )]);

        let results = parse(&html, 10).unwrap();
        assert_eq!(results[0].resolution.as_deref(), Some("1024x768"));
        assert_eq!(results[0].img_format.as_deref(), Some("JPEG"));
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
            "expected at least one image result from Bing"
        );
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.url.starts_with("http"));
            assert!(r.img_src.starts_with("http"));
        }
    }
}
