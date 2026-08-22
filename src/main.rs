use std::sync::Arc;
use std::time::Duration;

use metadata_search_engine_rs::{
    cache::{EngineLimits, build_response_cache},
    config::AppConfig,
    engines::{
        BingImagesEngine, BraveEngine, DuckDuckGoEngine, DuckDuckGoImagesEngine, GoogleImagesEngine,
        ImageSearchEngine, SearchEngine, SogouImagesEngine, StartpageEngine, YahooEngine,
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
    let timeout = Duration::from_millis(config.engine_timeout_ms);

    // Each engine gets its own client so cookies set by one engine's redirect
    // chains never leak into another engine's sessions.
    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(DuckDuckGoEngine::with_timeout(
            Arc::new(build_http_client()?),
            timeout,
        )),
        Arc::new(BraveEngine::with_timeout(
            Arc::new(build_http_client()?),
            timeout,
        )),
        Arc::new(StartpageEngine::with_timeout(
            Arc::new(build_http_client()?),
            timeout,
        )),
        Arc::new(YahooEngine::with_timeout(
            Arc::new(build_http_client()?),
            timeout,
        )),
    ];

    let image_engines: Vec<Arc<dyn ImageSearchEngine>> = vec![
        Arc::new(DuckDuckGoImagesEngine::with_timeout(
            Arc::new(build_http_client()?),
            timeout,
        )),
        Arc::new(BingImagesEngine::with_timeout(
            Arc::new(build_http_client()?),
            timeout,
        )),
        Arc::new(GoogleImagesEngine::with_timeout(
            Arc::new(build_http_client()?),
            timeout,
        )),
        Arc::new(SogouImagesEngine::with_timeout(
            Arc::new(build_http_client()?),
            timeout,
        )),
    ];

    let rate_limiter = (config.rate_limit_per_minute > 0).then(|| {
        metadata_search_engine_rs::server::ratelimit::RateLimiter::new(config.rate_limit_per_minute)
    });

    let state = Arc::new(AppState {
        engines,
        image_engines,
        results_per_engine: config.results_per_engine,
        max_results: config.max_results,
        cache: (config.cache_ttl_ms > 0)
            .then(|| build_response_cache(Duration::from_millis(config.cache_ttl_ms))),
        engine_limits: EngineLimits::new(
            config.engine_max_concurrency,
            Duration::from_millis(config.engine_cooldown_ms),
        ),
        allowed_origins: config.allowed_origins.clone(),
        rate_limiter,
    });

    let router = build_router(state);
    let addr = format!("{}:{}", config.host, config.port);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");

    // Wait for SIGINT/SIGTERM and stop accepting; in-flight requests drain.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, draining in-flight requests");
}
