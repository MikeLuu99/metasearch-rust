//! Basic fixed-window per-IP rate limiting.
//!
//! Protects the upstream engines from abuse: every uncached request fans out
//! to several upstream engines, so unbounded request rates amplify quickly.
//! The limiter counts requests per client IP within a one-minute window;
//! requests arriving without connection info (e.g. direct `tower::oneshot`
//! test calls) bypass the check.

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use moka::future::Cache;

/// Window length for the counters, in seconds.
const WINDOW_SECS: u64 = 60;

#[derive(Clone)]
pub struct RateLimiter {
    /// Key: `{ip}:{window_index}` -> hit count within that window. Entries
    /// expire two windows after their last touch, bounding memory per IP.
    hits: Cache<String, u64>,
    limit_per_minute: u64,
}

impl RateLimiter {
    pub fn new(limit_per_minute: u64) -> Self {
        Self {
            hits: Cache::builder()
                .time_to_live(Duration::from_secs(2 * WINDOW_SECS))
                .max_capacity(100_000)
                .build(),
            limit_per_minute,
        }
    }

    /// Record one hit for `ip` and report whether it is within the limit.
    ///
    /// The read-modify-write is not atomic, so concurrent bursts can slightly
    /// overshoot the limit — acceptable for protective purposes.
    async fn allow(&self, ip: &str) -> bool {
        let window = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / WINDOW_SECS)
            .unwrap_or(0);
        let key = format!("{ip}:{window}");

        let count = self.hits.get_with(key.clone(), async { 0u64 }).await + 1;
        self.hits.insert(key, count).await;

        count <= self.limit_per_minute
    }

    pub fn limited_response() -> Response {
        (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({ "error": "rate limit exceeded" })),
        )
            .into_response()
    }
}

/// Axum middleware enforcing the limiter. Requests without a
/// [`ConnectInfo<SocketAddr>`] extension (unit-test oneshot calls) pass
/// through unchecked.
pub async fn rate_limit(
    State(limiter): State<RateLimiter>,
    req: Request,
    next: axum::middleware::Next,
) -> Response {
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0.ip().to_string());

    if let Some(ip) = ip
        && !limiter.allow(&ip).await
    {
        return RateLimiter::limited_response();
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_up_to_limit_then_rejects() {
        let limiter = RateLimiter::new(3);

        for _ in 0..3 {
            assert!(limiter.allow("1.2.3.4").await);
        }
        assert!(!limiter.allow("1.2.3.4").await);

        // Other IPs have independent budgets.
        assert!(limiter.allow("5.6.7.8").await);
    }
}
