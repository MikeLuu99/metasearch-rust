use std::sync::Arc;

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use metadata_search_engine_rs::{
    engines::{BoxFuture, ImageSearchEngine, SearchEngine},
    error::EngineError,
    models::{ImageResult, SearchResult},
    server::{build_router, handlers::AppState},
};
use tower::util::ServiceExt;

struct MockEngine {
    name: &'static str,
    results: Vec<SearchResult>,
}

impl SearchEngine for MockEngine {
    fn name(&self) -> &'static str {
        self.name
    }

    fn search<'a>(
        &'a self,
        _query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
        let results = self.results.iter().take(max_results).cloned().collect();
        Box::pin(async move { Ok(results) })
    }
}

struct FailingEngine;

impl SearchEngine for FailingEngine {
    fn name(&self) -> &'static str {
        "failing"
    }

    fn search<'a>(
        &'a self,
        _query: &'a str,
        _max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
        Box::pin(async { Err(EngineError::Timeout { engine: "failing" }) })
    }
}

fn mock_result(title: &str, url: &str, engine: &str) -> SearchResult {
    SearchResult {
        title: title.to_string(),
        url: url.to_string(),
        snippet: Some(format!("{title} snippet")),
        source_engine: engine.to_string(),
    }
}

fn mock_image(title: &str, url: &str, img_src: &str, engine: &str) -> ImageResult {
    ImageResult {
        title: title.to_string(),
        url: url.to_string(),
        img_src: img_src.to_string(),
        thumbnail_src: Some(format!("{img_src}.thumb")),
        source: None,
        resolution: Some("1920x1080".to_string()),
        img_format: None,
        author: None,
        snippet: None,
        source_engine: engine.to_string(),
    }
}

struct MockImageEngine {
    name: &'static str,
    results: Vec<ImageResult>,
}

impl ImageSearchEngine for MockImageEngine {
    fn name(&self) -> &'static str {
        self.name
    }

    fn search_images<'a>(
        &'a self,
        _query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<ImageResult>, EngineError>> {
        let results = self.results.iter().take(max_results).cloned().collect();
        Box::pin(async move { Ok(results) })
    }
}

struct FailingImageEngine;

impl ImageSearchEngine for FailingImageEngine {
    fn name(&self) -> &'static str {
        "failing_image"
    }

    fn search_images<'a>(
        &'a self,
        _query: &'a str,
        _max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<ImageResult>, EngineError>> {
        Box::pin(async {
            Err(EngineError::Timeout {
                engine: "failing_image",
            })
        })
    }
}

fn build_test_router(engines: Vec<Arc<dyn SearchEngine>>) -> axum::Router {
    build_test_router_with_images(engines, vec![])
}

fn build_test_router_with_images(
    engines: Vec<Arc<dyn SearchEngine>>,
    image_engines: Vec<Arc<dyn ImageSearchEngine>>,
) -> axum::Router {
    let state = Arc::new(AppState {
        engines,
        image_engines,
        results_per_engine: 10,
        max_results: 10,
    });
    build_router(state)
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn test_health_returns_ok() {
    let router = build_test_router(vec![]);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn test_search_missing_query_returns_400() {
    let router = build_test_router(vec![]);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/search")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_search_empty_query_returns_400() {
    let router = build_test_router(vec![Arc::new(MockEngine {
        name: "mock",
        results: vec![],
    })]);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/search?q=")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(body["error"].as_str().unwrap().contains("empty"));
}

#[tokio::test]
async fn test_search_all_engines_fail_returns_503() {
    let router = build_test_router(vec![Arc::new(FailingEngine)]);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/search?q=rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let body = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("all engines failed")
    );
}

#[tokio::test]
async fn test_search_returns_aggregated_results() {
    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(MockEngine {
            name: "engine_a",
            results: vec![
                mock_result("Rust Lang", "https://rust-lang.org", "engine_a"),
                mock_result("Rust Book", "https://doc.rust-lang.org/book", "engine_a"),
            ],
        }),
        Arc::new(MockEngine {
            name: "engine_b",
            results: vec![
                mock_result("Rust Lang", "https://rust-lang.org", "engine_b"),
                mock_result(
                    "Wikipedia",
                    "https://en.wikipedia.org/wiki/Rust",
                    "engine_b",
                ),
            ],
        }),
    ];

    let router = build_test_router(engines);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/search?q=rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = json_body(response).await;

    assert_eq!(body["query"], "rust");
    let results = body["results"].as_array().unwrap();
    assert!(!results.is_empty());

    // rust-lang.org ranks first — returned by both engines at rank 1
    assert_eq!(results[0]["url"], "https://rust-lang.org");
    assert_eq!(results[0]["engines"].as_array().unwrap().len(), 2);

    assert_eq!(body["engines_queried"].as_array().unwrap().len(), 2);
    assert_eq!(body["engines_failed"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_search_partial_engine_failure_still_returns_results() {
    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(MockEngine {
            name: "working",
            results: vec![mock_result("Rust", "https://rust-lang.org", "working")],
        }),
        Arc::new(FailingEngine),
    ];

    let router = build_test_router(engines);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/search?q=rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = json_body(response).await;

    let results = body["results"].as_array().unwrap();
    assert!(!results.is_empty());

    let failed = body["engines_failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0], "failing");
}

#[tokio::test]
async fn test_search_respects_client_max_results_capped_by_config() {
    let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(MockEngine {
        name: "mock",
        results: (1..=20)
            .map(|i| mock_result(&format!("R{i}"), &format!("https://example{i}.com"), "mock"))
            .collect(),
    })];

    let state = Arc::new(AppState {
        engines,
        image_engines: vec![],
        results_per_engine: 10,
        max_results: 10,
    });
    let router = build_router(state);

    // Cap: request 50, get at most 10 (server config)
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search?q=rust&max_results=50")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["results"].as_array().unwrap().len(), 10);

    // Client can request fewer than the cap
    let response = router
        .oneshot(
            Request::builder()
                .uri("/search?q=rust&max_results=3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = json_body(response).await;
    assert_eq!(body["results"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn test_images_missing_query_returns_400() {
    let router = build_test_router(vec![]);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/images")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_images_no_engines_returns_503() {
    let router = build_test_router(vec![]);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/images?q=rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let body = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("no image engines configured")
    );
}

#[tokio::test]
async fn test_images_all_engines_fail_returns_503() {
    let router = build_test_router_with_images(vec![], vec![Arc::new(FailingImageEngine)]);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/images?q=rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn test_images_returns_aggregated_image_results() {
    let image_engines: Vec<Arc<dyn ImageSearchEngine>> = vec![Arc::new(MockImageEngine {
        name: "engine_a",
        results: vec![
            mock_image(
                "Rust Logo",
                "https://rust-lang.org/",
                "https://cdn.rust-lang.org/logo.png",
                "engine_a",
            ),
            mock_image(
                "Ferris",
                "https://rust-lang.org/ferris",
                "https://cdn.rust-lang.org/ferris.png",
                "engine_a",
            ),
        ],
    })];

    let router = build_test_router_with_images(vec![], image_engines);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/images?q=rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = json_body(response).await;

    assert_eq!(body["query"], "rust");
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["url"], "https://rust-lang.org/");
    assert_eq!(results[0]["img_src"], "https://cdn.rust-lang.org/logo.png");
    assert_eq!(
        results[0]["thumbnail_src"],
        "https://cdn.rust-lang.org/logo.png.thumb"
    );
    assert_eq!(results[0]["resolution"], "1920x1080");
    assert_eq!(body["engines_queried"].as_array().unwrap().len(), 1);
    assert_eq!(body["engines_failed"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_images_deduplicates_across_engines() {
    let image_engines: Vec<Arc<dyn ImageSearchEngine>> = vec![
        Arc::new(MockImageEngine {
            name: "engine_a",
            results: vec![mock_image(
                "Rust Logo",
                "https://rust-lang.org/",
                "https://cdn.rust-lang.org/logo.png",
                "engine_a",
            )],
        }),
        Arc::new(MockImageEngine {
            name: "engine_b",
            results: vec![mock_image(
                "Rust Logo",
                "https://rust-lang.org",
                "https://cdn.rust-lang.org/logo.png",
                "engine_b",
            )],
        }),
    ];

    let router = build_test_router_with_images(vec![], image_engines);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/images?q=rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = json_body(response).await;
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["engines"].as_array().unwrap().len(), 2);
}
