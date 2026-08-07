//! Run a single search engine from the CLI.
//!
//! Usage: `cargo run --example engines [engine] [query]`
//!
//! Supported engines: `duckduckgo`, `brave`, `startpage`, `yahoo`
//!
//! Examples:
//!   cargo run --example engines duckduckgo
//!   cargo run --example engines brave "rust programming"
//!
//! Unrecognized engine names fall back to Startpage. Results are printed with
//! title, URL and a truncated snippet.

use metadata_search_engine_rs::engines::{
    BraveEngine, DuckDuckGoEngine, SearchEngine, StartpageEngine, YahooEngine, build_http_client,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let engine_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "duckduckgo".to_string());
    let query = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "rust programming".to_string());
    let max_results = 5;

    let client = Arc::new(build_http_client()?);

    // 1. Use dynamic dispatch (dyn) to handle different engine types
    // 2. Map the Option<String> from engine_args to a specific engine
    let engine: Box<dyn SearchEngine> = match engine_name.as_str() {
        "duckduckgo" => Box::new(DuckDuckGoEngine::new(client.clone())),
        "brave" => Box::new(BraveEngine::new(client.clone())),
        "startpage" => Box::new(StartpageEngine::new(client.clone())),
        "yahoo" => Box::new(YahooEngine::new(client.clone())),
        _ => {
            println!("Engine not recognized. Defaulting to DuckDuckGo...");
            Box::new(StartpageEngine::new(client.clone()))
        }
    };

    println!("{engine_name} results:\n");
    println!("Searching for: {query:?}\n");

    match engine.search(&query, max_results).await {
        Ok(results) => {
            println!("Got {} result(s):\n", results.len());
            for (i, r) in results.iter().enumerate() {
                println!("  #{} {}", i + 1, r.title);
                println!("      {}", r.url);
                if let Some(snippet) = &r.snippet {
                    let len = snippet.len().min(120);
                    println!("      {}", &snippet[..len]);
                }
                println!();
            }
        }
        Err(e) => eprintln!("Error: {e}"),
    }

    Ok(())
}
