//! Run with: cargo run --example aggregator -- "your query"
//!
//! Fans out the query to all engines concurrently, then prints the
//! RRF-ranked aggregated results alongside which engines returned each URL.
use std::sync::Arc;

use metadata_search_engine_rs::{
    aggregator::{aggregate, query_all_engines},
    cache::EngineLimits,
    engines::{
        BraveEngine, DuckDuckGoEngine, SearchEngine, StartpageEngine, YahooEngine,
        build_http_client,
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rust programming".to_string());
    let results_per_engine = 10;
    let max_results = 10;

    println!("Querying all engines for: {query:?}\n");

    // One client per engine keeps each engine's cookie jar isolated.
    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(DuckDuckGoEngine::new(Arc::new(build_http_client()?))),
        Arc::new(BraveEngine::new(Arc::new(build_http_client()?))),
        Arc::new(StartpageEngine::new(Arc::new(build_http_client()?))),
        Arc::new(YahooEngine::new(Arc::new(build_http_client()?))),
    ];

    let (successes, failures) = query_all_engines(
        &engines,
        &EngineLimits::default(),
        &query,
        results_per_engine,
    )
    .await;

    if !failures.is_empty() {
        println!("Failed engines:");
        for (name, err) in &failures {
            println!("  {name}: {err}");
        }
        println!();
    }

    if successes.is_empty() {
        eprintln!("All engines failed — no results to aggregate.");
        return Ok(());
    }

    println!(
        "Results from {} engine(s): {}\n",
        successes.len(),
        successes
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let results = aggregate(successes, max_results);

    println!("Top {} aggregated results (RRF-ranked):\n", results.len());
    for (i, r) in results.iter().enumerate() {
        println!(
            "#{} [{:.4}] [{}]  {}",
            i + 1,
            r.score,
            r.engines.join(", "),
            r.title
        );
        println!("    {}", r.url);
        if let Some(snippet) = &r.snippet {
            println!("    {}", truncate(snippet, 120));
        }
        println!();
    }

    Ok(())
}

/// Truncate to at most `max` bytes without splitting a multi-byte character.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
