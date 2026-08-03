use std::collections::HashMap;
use std::sync::Arc;

use futures::future::join_all;

use crate::engines::SearchEngine;
use crate::error::EngineError;
use crate::models::{AggregatedResult, SearchResult};
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
    query: &str,
    max_results: usize,
) -> (Vec<(String, Vec<SearchResult>)>, Vec<(String, EngineError)>) {
    let futures: Vec<_> = engines
        .iter()
        .map(|engine| {
            let engine = Arc::clone(engine);
            let query = query.to_string();
            async move {
                let result = engine.search(&query, max_results).await;
                (engine.name().to_string(), result)
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
        for (index, result) in results.into_iter().enumerate() {
            let rank = index + 1; // 1-indexed for RRF
            let rrf_score = 1.0 / (RRF_K + rank as f64);

            let key = match normalizer::normalize(&result.url) {
                Some(k) => k,
                None => continue, // skip results with unparseable URLs
            };

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

        let (successes, failures) =
            query_all_engines(&engines, "rust programming language", 10).await;

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
