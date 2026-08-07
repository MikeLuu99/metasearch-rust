use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use metadata_search_engine_rs::{
    cache::{EngineLimits, build_response_cache},
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

/// An engine that counts how many times it is actually queried, so tests can
/// assert on cache hits and single-flight coalescing.
struct CountingEngine {
    calls: Arc<AtomicUsize>,
    results: Vec<SearchResult>,
}

impl SearchEngine for CountingEngine {
    fn name(&self) -> &'static str {
        "counting"
    }

    fn search<'a>(
        &'a self,
        _query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
        let calls = Arc::clone(&self.calls);
        let results = self.results.iter().take(max_results).cloned().collect();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(results)
        })
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
    build_test_state(engines, image_engines, 10, 10)
}

fn build_test_state(
    engines: Vec<Arc<dyn SearchEngine>>,
    image_engines: Vec<Arc<dyn ImageSearchEngine>>,
    results_per_engine: usize,
    max_results: usize,
) -> axum::Router {
    let state = Arc::new(AppState {
        engines,
        image_engines,
        results_per_engine,
        max_results,
        cache: Some(build_response_cache(Duration::from_secs(60))),
        engine_limits: EngineLimits::new(4, Duration::from_secs(30)),
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
        cache: Some(build_response_cache(Duration::from_secs(60))),
        engine_limits: EngineLimits::new(4, Duration::from_secs(30)),
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

#[tokio::test]
async fn test_repeated_search_is_served_from_cache() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(CountingEngine {
        calls: Arc::clone(&calls),
        results: vec![mock_result("Rust", "https://rust-lang.org", "counting")],
    })];

    let router = build_test_state(engines, vec![], 10, 10);

    for _ in 0..3 {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/search?q=rust")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_cache_key_is_case_insensitive() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(CountingEngine {
        calls: Arc::clone(&calls),
        results: vec![mock_result("Rust", "https://rust-lang.org", "counting")],
    })];

    let router = build_test_state(engines, vec![], 10, 10);

    for uri in ["/search?q=rust", "/search?q=Rust", "/search?q=%20RUST%20"] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_cache_does_not_mix_max_results() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(CountingEngine {
        calls: Arc::clone(&calls),
        results: (1..=10)
            .map(|i| {
                mock_result(
                    &format!("R{i}"),
                    &format!("https://example{i}.com"),
                    "counting",
                )
            })
            .collect(),
    })];

    let router = build_test_state(engines, vec![], 10, 10);

    // Different max_results must not share a cache entry.
    for uri in [
        "/search?q=rust&max_results=3",
        "/search?q=rust&max_results=10",
        "/search?q=rust&max_results=3",
    ] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_repeated_image_search_is_served_from_cache() {
    let calls = Arc::new(AtomicUsize::new(0));
    let image_engines: Vec<Arc<dyn ImageSearchEngine>> = vec![Arc::new(CountingImageEngine {
        calls: Arc::clone(&calls),
        results: vec![mock_image(
            "Rust Logo",
            "https://rust-lang.org",
            "https://cdn.rust-lang.org/logo.png",
            "counting",
        )],
    })];

    let router = build_test_state(vec![], image_engines, 10, 10);

    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/images?q=rust")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_concurrent_identical_queries_coalesce_to_one_engine_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(CountingEngine {
        calls: Arc::clone(&calls),
        results: vec![mock_result("Rust", "https://rust-lang.org", "counting")],
    })];

    let router = build_test_state(engines, vec![], 10, 10);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let router = router.clone();
        handles.push(tokio::spawn(async move {
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
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_failed_engine_is_cooled_down_across_requests() {
    struct FailingCountingEngine {
        calls: Arc<AtomicUsize>,
    }

    impl SearchEngine for FailingCountingEngine {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn search<'a>(
            &'a self,
            _query: &'a str,
            _max_results: usize,
        ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
            let calls = Arc::clone(&self.calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(EngineError::Timeout { engine: "failing" })
            })
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(FailingCountingEngine {
        calls: Arc::clone(&calls),
    })];

    let router = build_test_state(engines, vec![], 10, 10);

    for _ in 0..3 {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/search?q=rust")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // All engines failed, but the request still gets a 503 — the point is
        // the engine itself is only queried once.
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct CountingImageEngine {
    calls: Arc<AtomicUsize>,
    results: Vec<ImageResult>,
}

impl ImageSearchEngine for CountingImageEngine {
    fn name(&self) -> &'static str {
        "counting"
    }

    fn search_images<'a>(
        &'a self,
        _query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<ImageResult>, EngineError>> {
        let calls = Arc::clone(&self.calls);
        let results = self.results.iter().take(max_results).cloned().collect();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(results)
        })
    }
}
