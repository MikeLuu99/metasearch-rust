//! In-process performance layers: a TTL response cache and per-engine flow control.
//!
//! The response cache stores fully aggregated handler responses keyed by the
//! normalized query, so repeated searches never touch the upstream engines.
//! moka's `entry_by_ref` API coalesces concurrent misses (single-flight), so a
//! burst of identical queries triggers exactly one upstream fan-out.
//!
//! `EngineLimits` bounds concurrent requests against a single engine (a
//! semaphore) and short-circuits engines that failed recently (a negative
//! cache), so a dead engine stops costing every request its full timeout.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moka::future::Cache;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::EngineError;
use crate::models::{ImageSearchResponse, SearchResponse};

/// A fully aggregated handler response, cacheable as-is.
#[derive(Debug, Clone)]
pub enum CachedResponse {
    /// Response for `GET /search`.
    Search(SearchResponse),
    /// Response for `GET /images`.
    Image(ImageSearchResponse),
}

/// Build an in-memory TTL cache for aggregated responses.
pub fn build_response_cache(ttl: Duration) -> Cache<String, CachedResponse> {
    Cache::builder()
        .time_to_live(ttl)
        .max_capacity(10_000)
        .build()
}

/// Shared per-engine flow control.
///
/// Two independent mechanisms, both keyed by engine name:
/// - a [`Semaphore`] caps how many requests may be in flight against one engine;
/// - a failure cooldown skips engines that just failed, so a slow or dead
///   engine no longer blocks every request with its full timeout.
///
/// Cooldown is only extended by real query failures; a skip merely defers the
/// next attempt so a recovered engine is retried after the cooldown expires.
#[derive(Debug)]
pub struct EngineLimits {
    semaphores: Mutex<HashMap<&'static str, Arc<Semaphore>>>,
    cooldowns: Mutex<HashMap<&'static str, Instant>>,
    max_concurrency: usize,
    cooldown: Duration,
}

impl EngineLimits {
    pub fn new(max_concurrency: usize, cooldown: Duration) -> Self {
        Self {
            semaphores: Mutex::new(HashMap::new()),
            cooldowns: Mutex::new(HashMap::new()),
            max_concurrency,
            cooldown,
        }
    }

    /// Acquire a concurrency permit for the engine, or short-circuit with a
    /// [`EngineError::Cooldown`] if the engine is still in its failure window.
    ///
    /// The returned permit must be held for the duration of the engine call.
    pub async fn acquire(&self, name: &'static str) -> Result<OwnedSemaphorePermit, EngineError> {
        if self.in_cooldown(name) {
            return Err(EngineError::Cooldown { engine: name });
        }

        let semaphore = self
            .semaphores
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(name)
            .or_insert_with(|| Arc::new(Semaphore::new(self.max_concurrency)))
            .clone();

        // acquire_owned only errors if the semaphore was closed or created
        // with zero permits (e.g. a misconfigured limit of 0) — surface that
        // as an engine error instead of panicking in a request path.
        semaphore.acquire_owned().await.map_err(|_| {
            tracing::error!(engine = name, "engine semaphore unavailable (limit 0?)");
            EngineError::Unavailable { engine: name }
        })
    }

    /// Record that an engine call failed, starting its cooldown window.
    pub fn record_failure(&self, name: &'static str) {
        self.cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name, Instant::now());
    }

    fn in_cooldown(&self, name: &'static str) -> bool {
        let cooldowns = self
            .cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        matches!(
            cooldowns.get(&name),
            Some(started) if started.elapsed() < self.cooldown
        )
    }
}

impl Default for EngineLimits {
    fn default() -> Self {
        Self::new(4, Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_allows_concurrent_permits_up_to_the_limit() {
        let limits = EngineLimits::new(2, Duration::from_secs(30));

        let first = limits.acquire("engine").await.unwrap();
        let second = limits.acquire("engine").await.unwrap();

        // Third permit blocks while two are held.
        let timed_out = tokio::time::timeout(Duration::from_millis(50), limits.acquire("engine"))
            .await
            .is_err();
        assert!(timed_out);

        drop(first);
        let _third = tokio::time::timeout(Duration::from_millis(50), limits.acquire("engine"))
            .await
            .unwrap()
            .unwrap();
        drop(second);
    }

    #[tokio::test]
    async fn record_failure_starts_cooldown() {
        let limits = EngineLimits::new(2, Duration::from_secs(30));
        limits.record_failure("engine");

        assert!(matches!(
            limits.acquire("engine").await,
            Err(EngineError::Cooldown { engine: "engine" })
        ));
    }

    #[tokio::test]
    async fn cooldown_expires_after_duration() {
        let limits = EngineLimits::new(2, Duration::from_millis(20));
        limits.record_failure("engine");

        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = limits.acquire("engine").await.unwrap();
    }

    #[tokio::test]
    async fn cooldown_skips_do_not_extend_the_window() {
        let limits = EngineLimits::new(2, Duration::from_millis(20));
        limits.record_failure("engine");

        // Skip attempts don't touch the failure time, so the window still
        // expires after the original 20ms.
        for _ in 0..5 {
            let _ = limits.acquire("engine").await;
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = limits.acquire("engine").await.unwrap();
    }
}
