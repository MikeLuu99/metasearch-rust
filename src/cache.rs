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
    /// Per-engine cooldown start instant and the window duration in effect.
    cooldowns: Mutex<HashMap<&'static str, (Instant, Duration)>>,
    max_concurrency: usize,
    cooldown: Duration,
    /// Cooldown applied to rate-limit/access-denied failures (429/403): a
    /// longer window than [`Self::cooldown`], because retrying an actively
    /// throttling upstream on the standard window keeps the request signature
    /// hot and prolongs the block.
    rate_limit_cooldown: Duration,
    /// Ceiling for the escalating cooldown window
    /// (`ENGINE_COOLDOWN_MAX_MS`). SearXNG's `max_ban_time_on_fail`.
    max_cooldown: Duration,
    /// Consecutive failure count per engine, driving the escalation. Reset
    /// whenever the engine returns a success (SearXNG's `resume`).
    consecutive_failures: Mutex<HashMap<&'static str, u32>>,
    /// Optional global fan-out deadline (SearXNG-style bounded join): when it
    /// elapses, engines still in flight are reported as `EngineError::Timeout`
    /// and their requests are cancelled. `None` waits for every engine to
    /// finish or hit its own per-engine timeout.
    deadline: Option<Duration>,
}

impl EngineLimits {
    pub fn new(max_concurrency: usize, cooldown: Duration) -> Self {
        Self {
            semaphores: Mutex::new(HashMap::new()),
            cooldowns: Mutex::new(HashMap::new()),
            max_concurrency,
            cooldown,
            rate_limit_cooldown: Duration::from_secs(300),
            max_cooldown: Duration::from_secs(600),
            consecutive_failures: Mutex::new(HashMap::new()),
            deadline: None,
        }
    }

    /// Set the cooldown applied to rate-limit failures (429/403). Defaults to
    /// 5 minutes; a zero duration falls back to the standard cooldown.
    pub fn with_rate_limit_cooldown(mut self, rate_limit_cooldown: Duration) -> Self {
        self.rate_limit_cooldown = rate_limit_cooldown;
        self
    }

    /// Set the ceiling for the escalating cooldown window. Defaults to
    /// 10 minutes.
    pub fn with_max_cooldown(mut self, max_cooldown: Duration) -> Self {
        self.max_cooldown = max_cooldown;
        self
    }

    /// Set a global fan-out deadline. `None` (the default) keeps the previous
    /// wait-for-all semantics.
    pub fn with_deadline(mut self, deadline: Option<Duration>) -> Self {
        self.deadline = deadline;
        self
    }

    /// The configured global fan-out deadline, if any.
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    /// The cooldown window applied to rate-limit failures.
    pub fn rate_limit_cooldown(&self) -> Duration {
        self.rate_limit_cooldown
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

    /// Record that an engine call failed, starting its standard cooldown window.
    ///
    /// Escalation (SearXNG's `SuspendedStatus`): the window doubles with each
    /// *consecutive* failure — base, 2×, 4×, … capped at `max_cooldown` — so a
    /// flapping engine stops costing every request a timeout. A success resets
    /// the count (see [`Self::record_success`]). Rate-limit windows escalate
    /// from their own base the same way.
    pub fn record_failure(&self, name: &'static str) {
        let n = self.bump_failures(name);
        let window = self.escalate(self.cooldown, n);
        self.record_window(name, window);
    }

    /// Record an engine failure with an explicit base cooldown window. A zero
    /// duration falls back to the standard cooldown so a misconfigured limit
    /// cannot disable protection entirely. The window escalates with
    /// consecutive failures, capped at `max_cooldown`.
    pub fn record_failure_for(&self, name: &'static str, base_cooldown: Duration) {
        let n = self.bump_failures(name);
        let base = if base_cooldown.is_zero() {
            self.cooldown
        } else {
            base_cooldown
        };
        let window = self.escalate(base, n);
        self.record_window(name, window);
    }

    /// Record that an engine call succeeded, resetting its consecutive-failure
    /// count (SearXNG's `resume`).
    pub fn record_success(&self, name: &'static str) {
        self.consecutive_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(name);
    }

    /// Increment and return the consecutive failure count for the engine.
    fn bump_failures(&self, name: &'static str) -> u32 {
        let mut map = self
            .consecutive_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let n = map.entry(name).or_insert(0);
        *n += 1;
        *n
    }

    /// Escalate a base window: base × 2^(n-1) for the n-th consecutive
    /// failure, capped at `max_cooldown`. The first failure uses the base.
    fn escalate(&self, base: Duration, consecutive: u32) -> Duration {
        let multiplier = 1u32 << (consecutive - 1).min(16);
        let window = base.saturating_mul(multiplier);
        window.min(self.max_cooldown)
    }

    fn record_window(&self, name: &'static str, window: Duration) {
        self.cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name, (Instant::now(), window));
    }

    /// Window currently recorded for the engine, if any (test + debug aid).
    pub fn current_window(&self, name: &'static str) -> Option<Duration> {
        self.cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(name)
            .map(|(_, window)| *window)
    }

    fn in_cooldown(&self, name: &'static str) -> bool {
        let cooldowns = self
            .cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        matches!(
            cooldowns.get(name),
            Some((started, window)) if started.elapsed() < *window
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

    #[test]
    fn escalating_failure_windows_double_and_cap() {
        let limits = EngineLimits::new(2, Duration::from_millis(30))
            .with_max_cooldown(Duration::from_millis(100));

        // 1st failure: base window. 2nd: 2x. 3rd: 4x. 4th+: capped at max.
        limits.record_failure("e");
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(30)));
        limits.record_failure("e");
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(60)));
        limits.record_failure("e");
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(100)));
        limits.record_failure("e");
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(100)));
    }

    #[test]
    fn success_resets_the_escalation() {
        let limits = EngineLimits::new(2, Duration::from_millis(30))
            .with_max_cooldown(Duration::from_millis(100));

        limits.record_failure("e");
        limits.record_failure("e");
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(60)));

        limits.record_success("e");

        // Back to the base window after a recovery.
        limits.record_failure("e");
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(30)));
    }

    #[test]
    fn rate_limit_windows_escalate_from_their_own_base() {
        let limits = EngineLimits::new(2, Duration::from_millis(30))
            .with_rate_limit_cooldown(Duration::from_millis(200))
            .with_max_cooldown(Duration::from_millis(500));

        limits.record_failure_for("e", limits.rate_limit_cooldown());
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(200)));
        limits.record_failure_for("e", limits.rate_limit_cooldown());
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(400)));
        limits.record_failure_for("e", limits.rate_limit_cooldown());
        // 800ms would exceed the 500ms cap.
        assert_eq!(limits.current_window("e"), Some(Duration::from_millis(500)));
    }

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
