use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use std::sync::Arc;

use crate::{
    aggregator::{aggregate, aggregate_images, query_all_engines, query_all_image_engines},
    engines::{ImageSearchEngine, SearchEngine},
    error::AppError,
    models::{ImageSearchResponse, SearchQuery, SearchResponse},
};

pub struct AppState {
    pub engines: Vec<Arc<dyn SearchEngine>>,
    pub image_engines: Vec<Arc<dyn ImageSearchEngine>>,
    pub results_per_engine: usize,
    pub max_results: usize,
}

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    params: Result<Query<SearchQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<(StatusCode, Json<SearchResponse>), AppError> {
    let Query(params) = match params {
        Ok(p) => p,
        Err(_) => {
            return Err(AppError::bad_request("invalid query parameters"));
        }
    };

    let query = params.q.trim().to_string();

    if query.is_empty() {
        return Err(AppError::bad_request("query parameter 'q' cannot be empty"));
    }

    // Client may request fewer results, but never more than the server cap.
    let max_results = params
        .max_results
        .unwrap_or(state.max_results)
        .min(state.max_results)
        .max(1);

    let (successes, failures) =
        query_all_engines(&state.engines, &query, state.results_per_engine).await;

    let engines_queried: Vec<String> = state.engines.iter().map(|e| e.name().to_string()).collect();
    let engines_failed: Vec<String> = failures.iter().map(|(name, _)| name.clone()).collect();

    for (name, err) in &failures {
        tracing::warn!(engine = %name, error = %err, "engine query failed");
    }

    if successes.is_empty() {
        return Err(AppError::service_unavailable(
            "all engines failed to respond",
        ));
    }

    let results = aggregate(successes, max_results);

    Ok((
        StatusCode::OK,
        Json(SearchResponse {
            query,
            results,
            engines_queried,
            engines_failed,
        }),
    ))
}

pub async fn search_images(
    State(state): State<Arc<AppState>>,
    params: Result<Query<SearchQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<(StatusCode, Json<ImageSearchResponse>), AppError> {
    let Query(params) = match params {
        Ok(p) => p,
        Err(_) => {
            return Err(AppError::bad_request("invalid query parameters"));
        }
    };

    let query = params.q.trim().to_string();

    if query.is_empty() {
        return Err(AppError::bad_request("query parameter 'q' cannot be empty"));
    }

    if state.image_engines.is_empty() {
        return Err(AppError::service_unavailable("no image engines configured"));
    }

    // Client may request fewer results, but never more than the server cap.
    let max_results = params
        .max_results
        .unwrap_or(state.max_results)
        .min(state.max_results)
        .max(1);

    let (successes, failures) =
        query_all_image_engines(&state.image_engines, &query, state.results_per_engine).await;

    let engines_queried: Vec<String> = state
        .image_engines
        .iter()
        .map(|e| e.name().to_string())
        .collect();
    let engines_failed: Vec<String> = failures.iter().map(|(name, _)| name.clone()).collect();

    for (name, err) in &failures {
        tracing::warn!(engine = %name, error = %err, "image engine query failed");
    }

    if successes.is_empty() {
        return Err(AppError::service_unavailable(
            "all engines failed to respond",
        ));
    }

    let results = aggregate_images(successes, max_results);

    Ok((
        StatusCode::OK,
        Json(ImageSearchResponse {
            query,
            results,
            engines_queried,
            engines_failed,
        }),
    ))
}
