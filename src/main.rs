use std::sync::Arc;

use metadata_search_engine_rs::{
    config::AppConfig,
    engines::{
        BingImagesEngine, BraveEngine, DuckDuckGoEngine, GoogleImagesEngine, ImageSearchEngine,
        SearchEngine, SogouImagesEngine, StartpageEngine, YahooEngine, build_http_client,
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

    let timeout = std::time::Duration::from_millis(config.engine_timeout_ms);
    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(DuckDuckGoEngine::with_timeout(Arc::clone(&client), timeout)),
        Arc::new(BraveEngine::with_timeout(Arc::clone(&client), timeout)),
        Arc::new(StartpageEngine::with_timeout(Arc::clone(&client), timeout)),
        Arc::new(YahooEngine::with_timeout(Arc::clone(&client), timeout)),
    ];

    let image_engines: Vec<Arc<dyn ImageSearchEngine>> = vec![
        Arc::new(BingImagesEngine::with_timeout(Arc::clone(&client), timeout)),
        Arc::new(GoogleImagesEngine::with_timeout(
            Arc::clone(&client),
            timeout,
        )),
        Arc::new(SogouImagesEngine::with_timeout(
            Arc::clone(&client),
            timeout,
        )),
    ];

    let state = Arc::new(AppState {
        engines,
        image_engines,
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
