pub mod handlers;
pub mod ratelimit;

use std::sync::Arc;

use axum::{Router, middleware, routing::get};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

use handlers::AppState;

pub fn build_router(state: Arc<AppState>) -> Router {
    let rate_limiter = state.rate_limiter.clone();

    let router = Router::new()
        .route("/health", get(handlers::health))
        .route("/search", get(handlers::search))
        .route("/images", get(handlers::search_images))
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer(&state))
        .with_state(state);

    match rate_limiter {
        Some(limiter) => router.layer(middleware::from_fn_with_state(
            limiter,
            ratelimit::rate_limit,
        )),
        None => router,
    }
}

/// Restrict CORS to the configured origins when `ALLOWED_ORIGINS` is set.
/// Unset keeps the historical fully-permissive behavior.
fn cors_layer(state: &AppState) -> CorsLayer {
    let layer = CorsLayer::permissive();
    match &state.allowed_origins {
        Some(origins) if !origins.is_empty() => {
            let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
            if parsed.is_empty() {
                tracing::warn!("ALLOWED_ORIGINS contained no valid origins; leaving CORS permissive");
                layer
            } else {
                layer.allow_origin(AllowOrigin::list(parsed))
            }
        }
        _ => layer,
    }
}
