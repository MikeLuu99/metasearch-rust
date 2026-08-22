use std::collections::HashMap;
use std::sync::Arc;

use futures::future::join_all;

use crate::cache::EngineLimits;
use crate::engines::{ImageSearchEngine, SearchEngine};
use crate::error::EngineError;
use crate::models::{AggregatedImageResult, AggregatedResult, ImageResult, SearchResult};
use crate::normalizer;

// Standard RRF constant from the original paper (Cormack et al., 2009).
// Dampens the score gap between top and mid-ranked results so that
// cross-engine agreement can outweigh a single strong signal.
const RRF_K: f64 = 60.0;

/// Fan out a query to all engines concurrently.
///
/// Returns the collected raw results and a list of engine names that failed,
/// so the caller can include both in the response without aborting on partial failure.
pub async fn query_all_engines(
    engines: &[Arc<dyn SearchEngine>],
    limits: &EngineLimits,
    query: &str,
    max_results: usize,
) -> (Vec<(String, Vec<SearchResult>)>, Vec<(String, EngineError)>) {
    let futures: Vec<_> = engines
        .iter()
        .map(|engine| {
            let engine = Arc::clone(engine);
            let query = query.to_string();
            async move {
                let name = engine.name();
                let result = match limits.acquire(name).await {
                    Ok(_permit) => {
                        let res = engine.search(&query, max_results).await;
                        // Only a real failure cools the engine down; a cooldown
                        // skip defers the next attempt instead of pushing it out.
                        if res.is_err() {
                            limits.record_failure(name);
                        }
                        res
                    }
                    Err(e) => Err(e),
                };
                (name.to_string(), result)
            }
        })
        .collect();

    let outcomes = join_all(futures).await;

    let mut successes = Vec::new();
    let mut failures = Vec::new();

    for (name, result) in outcomes {
        match result {
            Ok(results) => successes.push((name, results)),
            Err(e) => failures.push((name, e)),
        }
    }

    (successes, failures)
}

/// Deduplicate and rank results from multiple engines using Reciprocal Rank Fusion.
///
/// RRF score per result: Σ 1 / (k + rank) across all engines that returned it.
/// Rank is 1-indexed. Higher score = more relevant.
pub fn aggregate(
    engine_results: Vec<(String, Vec<SearchResult>)>,
    max_results: usize,
) -> Vec<AggregatedResult> {
    let mut map: HashMap<String, AggregatedResult> = HashMap::new();

    for (engine_name, results) in engine_results {
        // Rank by position among *usable* results: entries skipped for
        // unparseable URLs must not consume rank positions and deflate the
        // scores of valid results behind them.
        let mut rank = 0usize;

        for result in results {
            let key = match normalizer::normalize(&result.url) {
                Some(k) => k,
                None => {
                    tracing::debug!(engine = %engine_name, url = %result.url, "skipping result with unparseable url");
                    continue;
                }
            };

            rank += 1;
            let rrf_score = 1.0 / (RRF_K + rank as f64);

            match map.get_mut(&key) {
                Some(existing) => {
                    existing.score += rrf_score;
                    if !existing.engines.contains(&engine_name) {
                        existing.engines.push(engine_name.clone());
                    }
                    // Prefer a longer snippet if we don't have one yet
                    if existing.snippet.is_none() && result.snippet.is_some() {
                        existing.snippet = result.snippet;
                    }
                }
                None => {
                    map.insert(
                        key,
                        AggregatedResult {
                            title: result.title,
                            url: result.url,
                            snippet: result.snippet,
                            engines: vec![engine_name.clone()],
                            score: rrf_score,
                        },
                    );
                }
            }
        }
    }

    let mut ranked: Vec<AggregatedResult> = map.into_values().collect();

    // Primary: score descending. Secondary: title ascending for stable ordering on ties.
    ranked.sort_unstable_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.cmp(&b.title))
    });

    ranked.truncate(max_results);
    ranked
}

/// Fan out a query to all image engines concurrently.
///
/// Same partial-failure semantics as [`query_all_engines`].
pub async fn query_all_image_engines(
    engines: &[Arc<dyn ImageSearchEngine>],
    limits: &EngineLimits,
    query: &str,
    max_results: usize,
) -> (Vec<(String, Vec<ImageResult>)>, Vec<(String, EngineError)>) {
    let futures: Vec<_> = engines
        .iter()
        .map(|engine| {
            let engine = Arc::clone(engine);
            let query = query.to_string();
            async move {
                let name = engine.name();
                let result = match limits.acquire(name).await {
                    Ok(_permit) => {
                        let res = engine.search_images(&query, max_results).await;
                        if res.is_err() {
                            limits.record_failure(name);
                        }
                        res
                    }
                    Err(e) => Err(e),
                };
                (name.to_string(), result)
            }
        })
        .collect();

    let outcomes = join_all(futures).await;

    let mut successes = Vec::new();
    let mut failures = Vec::new();

    for (name, result) in outcomes {
        match result {
            Ok(results) => successes.push((name, results)),
            Err(e) => failures.push((name, e)),
        }
    }

    (successes, failures)
}

/// Deduplicate and rank image results across engines using RRF.
///
/// Image identity is the hosting page (normalized) *plus* the image file URL,
/// mirroring SearXNG's `template|url|img_src` image result hash: two results
/// from the same page but different images are distinct.
pub fn aggregate_images(
    engine_results: Vec<(String, Vec<ImageResult>)>,
    max_results: usize,
) -> Vec<AggregatedImageResult> {
    let mut map: HashMap<String, AggregatedImageResult> = HashMap::new();

    for (engine_name, results) in engine_results {
        // Rank by position among *usable* results, mirroring `aggregate`.
        let mut rank = 0usize;

        for result in results {
            let key = match image_key(&result) {
                Some(k) => k,
                None => {
                    tracing::debug!(engine = %engine_name, url = %result.url, "skipping image result with unparseable url");
                    continue;
                }
            };

            rank += 1;
            let rrf_score = 1.0 / (RRF_K + rank as f64);

            match map.get_mut(&key) {
                Some(existing) => {
                    existing.score += rrf_score;
                    if !existing.engines.contains(&engine_name) {
                        existing.engines.push(engine_name.clone());
                    }
                    // Fill in metadata missing from the first-seen result
                    if existing.thumbnail_src.is_none() {
                        existing.thumbnail_src = result.thumbnail_src;
                    }
                    if existing.source.is_none() {
                        existing.source = result.source;
                    }
                    if existing.resolution.is_none() {
                        existing.resolution = result.resolution;
                    }
                    if existing.img_format.is_none() {
                        existing.img_format = result.img_format;
                    }
                    if existing.author.is_none() {
                        existing.author = result.author;
                    }
                    if existing.snippet.is_none() {
                        existing.snippet = result.snippet;
                    }
                }
                None => {
                    map.insert(
                        key,
                        AggregatedImageResult {
                            title: result.title,
                            url: result.url,
                            img_src: result.img_src,
                            thumbnail_src: result.thumbnail_src,
                            source: result.source,
                            resolution: result.resolution,
                            img_format: result.img_format,
                            author: result.author,
                            snippet: result.snippet,
                            engines: vec![engine_name.clone()],
                            score: rrf_score,
                        },
                    );
                }
            }
        }
    }

    let mut ranked: Vec<AggregatedImageResult> = map.into_values().collect();

    // Primary: score descending. Secondary: title ascending for stable ordering on ties.
    ranked.sort_unstable_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.cmp(&b.title))
    });

    ranked.truncate(max_results);
    ranked
}

fn image_key(result: &ImageResult) -> Option<String> {
    let page = normalizer::normalize(&result.url)?;
    // Normalize the image URL too so scheme/tracking variants of the same
    // file merge instead of counting as distinct images.
    let img = normalizer::normalize(&result.img_src).unwrap_or_else(|| result.img_src.clone());
    Some(format!("{page}|{img}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(url: &str, engine: &str, title: &str) -> SearchResult {
        SearchResult {
            title: title.to_string(),
            url: url.to_string(),
            snippet: None,
            source_engine: engine.to_string(),
        }
    }

    fn make_image(url: &str, img_src: &str, engine: &str, title: &str) -> ImageResult {
        ImageResult {
            title: title.to_string(),
            url: url.to_string(),
            img_src: img_src.to_string(),
            thumbnail_src: None,
            source: None,
            resolution: None,
            img_format: None,
            author: None,
            snippet: None,
            source_engine: engine.to_string(),
        }
    }

    #[test]
    fn test_rrf_cross_engine_agreement_boosts_score() {
        let engine_results = vec![
            (
                "ddg".to_string(),
                vec![make_result("https://example.com", "ddg", "Example")],
            ),
            (
                "brave".to_string(),
                vec![make_result("https://example.com", "brave", "Example")],
            ),
        ];
        let results = aggregate(engine_results, 10);

        assert_eq!(results.len(), 1);
        // Score should be 1/(60+1) + 1/(60+1) — both ranked #1
        let expected = 2.0 / 61.0;
        assert!((results[0].score - expected).abs() < 1e-10);
        assert_eq!(results[0].engines.len(), 2);
    }

    #[test]
    fn test_rrf_rank1_beats_rank5_single_engine() {
        let engine_results = vec![(
            "ddg".to_string(),
            vec![
                make_result("https://rank1.com", "ddg", "Rank 1"),
                make_result("https://rank2.com", "ddg", "Rank 2"),
                make_result("https://rank3.com", "ddg", "Rank 3"),
                make_result("https://rank4.com", "ddg", "Rank 4"),
                make_result("https://rank5.com", "ddg", "Rank 5"),
            ],
        )];
        let results = aggregate(engine_results, 10);

        assert_eq!(results[0].url, "https://rank1.com");
        assert!(results[0].score > results[4].score);
    }

    #[test]
    fn test_deduplication_by_normalized_url() {
        let engine_results = vec![
            (
                "ddg".to_string(),
                vec![make_result("https://example.com/page/", "ddg", "Page")],
            ),
            (
                "brave".to_string(),
                vec![make_result("https://example.com/page", "brave", "Page")],
            ),
        ];
        let results = aggregate(engine_results, 10);

        // Trailing slash difference should be normalized — one deduplicated result
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].engines.len(), 2);
    }

    #[test]
    fn test_skips_unparseable_urls() {
        let engine_results = vec![(
            "ddg".to_string(),
            vec![
                make_result("not a url", "ddg", "Bad"),
                make_result("https://valid.com", "ddg", "Good"),
            ],
        )];
        let results = aggregate(engine_results, 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://valid.com");
    }

    #[test]
    fn test_respects_max_results() {
        let engine_results = vec![(
            "ddg".to_string(),
            (1..=10)
                .map(|i| {
                    make_result(
                        &format!("https://example{i}.com"),
                        "ddg",
                        &format!("Result {i}"),
                    )
                })
                .collect(),
        )];
        let results = aggregate(engine_results, 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_snippet_preference_from_secondary_engine() {
        let engine_results = vec![
            (
                "ddg".to_string(),
                vec![SearchResult {
                    title: "Page".to_string(),
                    url: "https://example.com".to_string(),
                    snippet: None,
                    source_engine: "ddg".to_string(),
                }],
            ),
            (
                "brave".to_string(),
                vec![SearchResult {
                    title: "Page".to_string(),
                    url: "https://example.com".to_string(),
                    snippet: Some("A useful snippet.".to_string()),
                    source_engine: "brave".to_string(),
                }],
            ),
        ];
        let results = aggregate(engine_results, 10);

        assert_eq!(results[0].snippet, Some("A useful snippet.".to_string()));
    }

    #[test]
    fn test_image_dedup_by_page_and_img_src() {
        let engine_results = vec![
            (
                "ddg".to_string(),
                vec![make_image(
                    "https://example.com/page",
                    "https://cdn.com/a.jpg",
                    "ddg",
                    "A",
                )],
            ),
            (
                "bing".to_string(),
                vec![make_image(
                    "https://example.com/page/",
                    "https://cdn.com/a.jpg",
                    "bing",
                    "A",
                )],
            ),
        ];
        let results = aggregate_images(engine_results, 10);

        // Trailing slash normalized — one merged result from both engines
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].engines.len(), 2);
        let expected = 2.0 / 61.0;
        assert!((results[0].score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_same_page_different_image_not_merged() {
        let engine_results = vec![
            (
                "ddg".to_string(),
                vec![make_image(
                    "https://example.com/page",
                    "https://cdn.com/a.jpg",
                    "ddg",
                    "A",
                )],
            ),
            (
                "bing".to_string(),
                vec![make_image(
                    "https://example.com/page",
                    "https://cdn.com/b.jpg",
                    "bing",
                    "B",
                )],
            ),
        ];
        let results = aggregate_images(engine_results, 10);

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_image_rrf_rank1_beats_rank2() {
        let engine_results = vec![(
            "ddg".to_string(),
            vec![
                make_image("https://page1.com", "https://cdn1.com/i.jpg", "ddg", "One"),
                make_image("https://page2.com", "https://cdn2.com/i.jpg", "ddg", "Two"),
            ],
        )];
        let results = aggregate_images(engine_results, 10);

        assert_eq!(results[0].url, "https://page1.com");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_image_skips_unparseable_page_url() {
        let engine_results = vec![(
            "ddg".to_string(),
            vec![
                make_image("not a url", "https://cdn.com/i.jpg", "ddg", "Bad"),
                make_image("https://valid.com", "https://cdn.com/i.jpg", "ddg", "Good"),
            ],
        )];
        let results = aggregate_images(engine_results, 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://valid.com");
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_aggregation() {
        use crate::engines::{BraveEngine, DuckDuckGoEngine, StartpageEngine};

        let client = Arc::new(crate::engines::build_http_client().unwrap());

        let engines: Vec<Arc<dyn SearchEngine>> = vec![
            Arc::new(DuckDuckGoEngine::new(Arc::clone(&client))),
            Arc::new(BraveEngine::new(Arc::clone(&client))),
            Arc::new(StartpageEngine::new(Arc::clone(&client))),
        ];

        let (successes, failures) = query_all_engines(
            &engines,
            &EngineLimits::default(),
            "rust programming language",
            10,
        )
        .await;

        println!("Engines succeeded: {}", successes.len());
        println!("Engines failed:    {}", failures.len());
        for (name, err) in &failures {
            println!("  FAILED {name}: {err}");
        }

        let results = aggregate(successes, 10);

        println!("\nTop {} aggregated results:", results.len());
        for (i, r) in results.iter().enumerate() {
            println!(
                "\n  #{} [{:.4}] [{}] {}",
                i + 1,
                r.score,
                r.engines.join(", "),
                r.title
            );
            println!("      {}", r.url);
            if let Some(s) = &r.snippet {
                println!("      snippet: {}", &s[..s.len().min(100)]);
            }
        }

        assert!(!results.is_empty(), "expected aggregated results");
        // At least one result should appear in both engines — the web agrees on something
        let cross_engine = results.iter().filter(|r| r.engines.len() > 1).count();
        println!("\nResults from both engines: {cross_engine}");
        assert!(
            cross_engine > 0,
            "expected at least one result from both engines"
        );
    }
}
