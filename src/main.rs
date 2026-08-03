use std::sync::Arc;

use metadata_search_engine_rs::{
    config::AppConfig,
    engines::{
        BraveEngine, DuckDuckGoEngine, SearchEngine, StartpageEngine, YahooEngine,
        build_http_client,
    },
    server::{build_router, handlers::AppState},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("metadata_search_engine_rs=debug".parse()?),
        )
        .init();

    let config = AppConfig::from_env();

    let client = Arc::new(build_http_client()?);

    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(DuckDuckGoEngine {
            client: Arc::clone(&client),
        }),
        Arc::new(BraveEngine {
            client: Arc::clone(&client),
        }),
        Arc::new(StartpageEngine {
            client: Arc::clone(&client),
        }),
        Arc::new(YahooEngine {
            client: Arc::clone(&client),
        }),
    ];

    let state = Arc::new(AppState {
        engines,
        results_per_engine: config.results_per_engine,
        max_results: config.max_results,
    });

    let router = build_router(state);
    let addr = format!("0.0.0.0:{}", config.port);

    tracing::info!("listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
