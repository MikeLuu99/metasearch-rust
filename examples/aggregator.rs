/// Run with: cargo run --example aggregator -- "your query"
///
/// Fans out the query to all engines concurrently, then prints the
/// RRF-ranked aggregated results alongside which engines returned each URL.
use std::sync::Arc;

use metadata_search_engine_rs::{
    aggregator::{aggregate, query_all_engines},
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

    let client = Arc::new(build_http_client()?);

    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(DuckDuckGoEngine::new(Arc::clone(&client))),
        Arc::new(BraveEngine::new(Arc::clone(&client))),
        Arc::new(StartpageEngine::new(Arc::clone(&client))),
        Arc::new(YahooEngine::new(Arc::clone(&client))),
    ];

    let (successes, failures) = query_all_engines(&engines, &query, results_per_engine).await;

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
            println!("    {}", &snippet[..snippet.len().min(120)]);
        }
        println!();
    }

    Ok(())
}
