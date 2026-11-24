# metasearch-rs

A SearXNG-style metadata search engine written in Rust. Fans out queries to multiple search engines concurrently, scrapes their HTML results, deduplicates by normalized URL, and ranks using Reciprocal Rank Fusion (RRF). Also supports image search through a dedicated `/images` endpoint.

## How it works

1. A search request arrives at `GET /search?q=<query>`
2. The query is sent concurrently to DuckDuckGo, Brave, Startpage, and Yahoo via `reqwest`
3. Each engine parses the HTML response with `scraper` (CSS selectors over Mozilla's html5ever)
4. Results are deduplicated by normalized URL (tracking params stripped, locale prefixes removed, query params sorted)
5. Duplicate URLs are merged and scored with RRF (`score = Σ 1/(60 + rank)` across engines) — pages returned by multiple engines rank higher
6. The top results are returned as JSON

Image search follows the same pipeline on `GET /images?q=<query>`: the query is fanned out to image engines (Bing Images via its `/images/async` endpoint, Google Images via its internal `async=_fmt:json` API, and Sogou Images via its embedded `__INITIAL_STATE__` JSON), results carry the hosting page URL, the full image URL and a thumbnail preview, and are deduplicated by `normalized page URL + image URL` before RRF ranking.

## Requirements

- Rust 1.85+ (the crate uses edition 2024)
- Cargo

## Installation

### As a library

```bash
cargo add metadata-search-engine-rs
```

Or add manually to `Cargo.toml`:

```toml
[dependencies]
metadata-search-engine-rs = "0.3"
```

### As a server (from source)

```bash
git clone https://github.com/MikeLuu99/metasearch-rust
cd metadata-search-engine-rs
cargo build --release
```

## Running

```bash
cargo run
```

```bash
PORT=8080 MAX_RESULTS=20 cargo run --release
```

Enable debug logging:

```bash
RUST_LOG=debug cargo run
```

## Examples

Add to your `Cargo.toml`:

```toml
[dependencies]
metadata-search-engine-rs = "0.3"
tokio = { version = "1", features = ["full"] }
```

### Query a single engine

```rust
use std::sync::Arc;
use metadata_search_engine_rs::engines::{DuckDuckGoEngine, SearchEngine, build_http_client};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Arc::new(build_http_client()?);
    let engine = DuckDuckGoEngine::new(client);

    let results = engine.search("rust programming", 5).await?;
    for r in results {
        println!("{}\n  {}", r.title, r.url);
    }
    Ok(())
}
```

### Fan out to all engines and get RRF-ranked results

```rust
use std::sync::Arc;
use metadata_search_engine_rs::{
    aggregator::{aggregate, query_all_engines},
    cache::EngineLimits,
    engines::{BraveEngine, DuckDuckGoEngine, SearchEngine, StartpageEngine, YahooEngine, build_http_client},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Arc::new(build_http_client()?);
    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(DuckDuckGoEngine::new(Arc::clone(&client))),
        Arc::new(BraveEngine::new(Arc::clone(&client))),
        Arc::new(StartpageEngine::new(Arc::clone(&client))),
        Arc::new(YahooEngine::new(Arc::clone(&client))),
    ];

    let (successes, failures) =
        query_all_engines(&engines, &EngineLimits::default(), "rust programming", 10).await;
    for (name, err) in &failures {
        eprintln!("engine {name} failed: {err}");
    }

    let results = aggregate(successes, 10);
    for r in &results {
        println!("[{:.3}] ({}) {}", r.score, r.engines.join(", "), r.title);
        println!("        {}", r.url);
    }
    Ok(())
}
```

### Use only specific engines

```rust
use std::sync::Arc;
use metadata_search_engine_rs::{
    aggregator::{aggregate, query_all_engines},
    cache::EngineLimits,
    engines::{BraveEngine, DuckDuckGoEngine, SearchEngine, build_http_client},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Arc::new(build_http_client()?);
    let engines: Vec<Arc<dyn SearchEngine>> = vec![
        Arc::new(DuckDuckGoEngine::new(Arc::clone(&client))),
        Arc::new(BraveEngine::new(Arc::clone(&client))),
    ];

    let (successes, _) = query_all_engines(&engines, &EngineLimits::default(), "tokio async rust", 5).await;
    for r in aggregate(successes, 5) {
        println!("{} — {}", r.title, r.url);
        if let Some(snippet) = r.snippet {
            println!("  {snippet}");
        }
    }
    Ok(())
}
```

## API

### `GET /health`

```bash
curl http://localhost:3000/health
```

```json
{ "status": "ok" }
```

### `GET /search?q=<query>`

```bash
curl "http://localhost:3000/search?q=rust"
```

```json
{
  "query": "rust",
  "results": [
    {
      "title": "Rust Programming Language",
      "url": "https://rust-lang.org/",
      "snippet": "A language empowering everyone to build reliable and efficient software.",
      "engines": ["duckduckgo", "brave", "startpage", "yahoo"],
      "score": 0.049
    }
  ],
  "engines_queried": ["duckduckgo", "brave", "startpage", "yahoo"],
  "engines_failed": []
}
```

**Error responses:**

| Case             | Status | Body                                               |
| ---------------- | ------ | -------------------------------------------------- |
| Missing `q`      | 400    | `{"error": "invalid query parameters"}`              |
| Empty `q`        | 400    | `{"error": "query parameter 'q' cannot be empty"}` |
| All engines fail | 503    | `{"error": "all engines failed to respond"}`       |

### `GET /images?q=<query>`

```bash
curl "http://localhost:3000/images?q=rust%20logo"
```

```json
{
  "query": "rust logo",
  "results": [
    {
      "title": "Rust Logo",
      "url": "https://rust-lang.org/",
      "img_src": "https://cdn.rust-lang.org/logo.png",
      "thumbnail_src": "https://cdn.rust-lang.org/thumb.png",
      "source": "rust-lang.org",
      "resolution": "3840x2160",
      "engines": ["bing", "google", "sogou"],
      "score": 0.049
    }
  ],
  "engines_queried": ["bing", "google", "sogou"],
  "engines_failed": []
}
```

`url` is the page hosting the image, `img_src` is the full-resolution image file, and `thumbnail_src` is a smaller preview. Image results are deduplicated by normalized hosting page URL + `img_src`, mirroring SearXNG's `template|url|img_src` image result hash.

**Error responses:** same semantics as `/search`, plus 503 `{"error": "no image engines configured"}` when no image engines are wired in.

## Load testing

A [Locust](https://locust.io/) load test lives in `loadtest/` (excluded from
the published crate). [uv](https://docs.astral.sh/uv/) manages its deps.

> ⚠️ `/search` and `/images` hit real upstream engines — keep the user count
> modest to avoid rate-limiting your IP. The response cache (`CACHE_TTL_MS`)
> and `ENGINE_MAX_CONCURRENCY` reduce upstream load.

```bash
cargo run --release                     # 1. start the server
cd loadtest && uv sync && cd ..         # 2. install deps (first time)

# 3. Web UI (localhost:8089), or headless:
uv run --directory loadtest locust -f locustfile.py --config locust.conf
uv run --directory loadtest locust -f locustfile.py --headless \
    --host http://localhost:3000 --users 50 --spawn-rate 5 --run-time 2m
```

Task mix is 70% `/search`, 20% `/images`, 10% `/health`; the query pool is
embedded in `loadtest/locustfile.py` (`_QUERIES`). Add `--csv results` for CSV
output.

## Benchmarks vs SearXNG

Head-to-head measured on the same host, same IP, same upstream engines.

**Environment:** Apple M4 (10 cores), macOS 15.7.2, wired to the real internet.
`metasearch-rs` v0.3.3 release build; SearXNG docker image `2026.11.23+a7163d091`
(official image, uwsgi, default engine set). metasearch-rs ran with
`RATE_LIMIT_PER_MINUTE=0` and its default engine set (DDG, Brave, Startpage,
Yahoo). Latency harness: stdlib Python, sequential 20-distinct-query run;
throughput: 5-query pool, fixed concurrency, 20–30 s. Percentiles from raw
per-request timings.

### 1. Cold latency — uncached end-to-end `/search` (upstream-dominated)

metasearch-rs: 50 distinct queries, one request each, sequential, uncached.
SearXNG: same protocol, 20-query clean run taken before any load; later
re-measurement attempts were invalidated because the shared IP had been
temporarily blocked by SearXNG's upstreams (see §2), so its p90/p99 could not
be re-measured — its clean p95/max (344/428 ms) bracket where they fall.

| Server          | p50    | p90    | p95     | p99      | max      | success |
| --------------- | ------ | ------ | ------- | -------- | -------- | ------- |
| SearXNG         | 260 ms | n/a¹   | 344 ms  | n/a¹     | 428 ms   | 100%    |
| metasearch-rs   | 407 ms | 478 ms | 575 ms  | 8,011 ms | 8,011 ms | 100%    |

**Why is metasearch-rs's p50 slower?** Because response time ≈ *slowest
engine*, and the implementations wait on different engine sets — not because of
the language:

- The aggregator waits for **all** engines before responding
  (`join_all`, src/aggregator.rs:51), so every request samples the maximum of
  its engines' latencies. The max of 4 engines (metasearch-rs: DDG `/html/`,
  Brave, Startpage, Yahoo) runs higher than the max of 2 (the official SearXNG
  docker image's default set: DDG POST + Google CSE) — mathematically and
  measurably.
- **Update:** the benchmark below was measured *before* a global fan-out
  deadline was added. Since then the aggregator races the fan-out against a
  deadline (`SEARCH_DEADLINE_MS`, default 5 s, `0` disables) SearXNG-style:
  when it elapses, engines still in flight are reported in `engines_failed` as
  `Timeout` and the response returns with the engines that finished. This caps
  the worst case at 5 s instead of `ENGINE_TIMEOUT_MS` (8 s), and because
  deadline cuts don't trigger the failure cooldown, slow-but-alive engines are
  retried on the next request.
- Direct per-engine probes: Brave responded in ~1.0 s (vs ~200–300 ms for the
  others when healthy). Brave alone drags the max-of-4 distribution up.
- The p99 = 8,011 ms is exactly `ENGINE_TIMEOUT_MS` (8 s): DDG's `/html/`
  endpoint occasionally "slow-drips" (a direct probe hung 75 s under repeated
  hits), and the per-engine timeout converts that stall into a bounded 8 s
  wait. SearXNG's equivalent cap is its own per-engine timeout (~3 s), which is
  what bounded its blocked-run tail at 3,025 ms.
- Server-side work (parse, dedup, RRF) is ~1 ms of the ~400 ms request for
  both. Cold latency is upstream engine variance, not runtime overhead.

### 2. Throughput — uncached `/search` (upstream-bound)

4 concurrent clients, 5-query pool, 20–30 s, both servers uncached:

| Server        | RPS   | p50   | outcome                                    |
| ------------- | ----- | ----- | ------------------------------------------ |
| metasearch-rs | 21.2  | 81 ms | 43% success — engines began 403-ing mid-run |
| SearXNG       | 382*  | 8 ms  | engines suspended → served empty shells     |

\* SearXNG's 382 RPS is HTTP 200s with `results: []` — its engine-suspension
mechanism had suspended DDG/Google (403 "access denied" / "unusual traffic")
and kept serving instant empty responses. metasearch-rs kept paying real
upstream latency until its per-engine cooldown engaged, hence its lower RPS and
error rate. **Takeaway:** with the response cache disabled, throughput is
bounded by upstream anti-bot defenses, not by either implementation — both got
the shared IP throttled within ~30 s at just 4 concurrent clients. (The two
runs also confound each other: whichever runs second inherits an already-hot
IP. Read this row as "uncached search does not scale," not as a winner.)

### 3. Pure HTTP overhead — `/health` vs `/healthz` (no upstream calls)

`ab -n 20000 -c 32`, static endpoint, isolates the server stacks:

| Server          | RPS       | p50   | p99   |
| --------------- | --------- | ----- | ----- |
| metasearch-rs   | **48,191** | 1 ms  | 2 ms  |
| SearXNG         | 2,938     | 10 ms | 16 ms |

**~16× throughput, ~10× p50 latency advantage** for Axum/Tokio over the SearXNG
uwsgi/Flask stack. (Caveat: the official SearXNG image runs uwsgi with
conservative worker defaults tuned for scraping, not for static-endpoint RPS.)

### 4. Throughput — metasearch-rs with response cache (`CACHE_TTL_MS=60000`)

Queries served from the in-process cache (single-flight coalesced). SearXNG has
no equivalent without an external Redis result cache, so this is metasearch-rs
only:

| Concurrency | RPS    | p50   | p95   | success |
| ----------- | ------ | ----- | ----- | ------- |
| 4           | 7,258  | 0.5 ms | 0.7 ms | 100%    |
| 32          | 6,786  | 4.0 ms | 7.7 ms | 100%    |

Zero errors at both concurrency levels — repeated queries never touch upstream
engines, which also eliminates the IP-blocking failure mode that caps uncached
throughput.

### 5. Memory footprint (steady state)

| Server        | RSS     |
| ------------- | ------- |
| metasearch-rs | ~19 MB  |
| SearXNG       | ~123 MB |

### Summary

| Metric                     | metasearch-rs | SearXNG | Winner         |
| -------------------------- | ------------- | ------- | -------------- |
| Cold (uncached) p50        | 407 ms        | 260 ms  | SearXNG        |
| Uncached throughput        | upstream-bound | upstream-bound | tie (both collapse) |
| Static HTTP RPS            | 48,191        | 2,938   | metasearch-rs  |
| Cached throughput          | 6,786–7,258 RPS | n/a (needs Redis) | metasearch-rs |
| Memory                     | ~19 MB        | ~123 MB | metasearch-rs  |

The Rust implementation's advantage is in the serving layer (throughput,
latency under concurrency, memory, zero-dependency caching). The scraping path
is upstream-bound for both, where SearXNG's larger engine pool gives it better
cold p50 and its engine-suspension gives it graceful degradation under blocks.

## Tests

```bash
# All unit tests
cargo test

# Specific module
cargo test normalizer
cargo test aggregator
cargo test engines::duckduckgo
cargo test engines::brave
cargo test engines::startpage
cargo test engines::yahoo
cargo test server::handlers

# Live tests (hit real search engines — requires internet)
cargo test -- --ignored test_live
```

Live tests are marked `#[ignore]` so they don't run in CI by default. Run them manually to verify HTML selectors still work against the real sites.

## Terminal UI

A ratatui-based TUI is available as a [crate](https://crates.io/crates/search-tui) to install here. Access the code via [github](https://github.com/MikeLuu99/search-tui)

![stx TUI](assets/tui_screenshots.png)

## Adding a new search engine

1. Create `src/engines/<name>.rs`
2. Define a struct holding `Arc<reqwest::Client>`
3. Implement the `SearchEngine` trait:

```rust
impl SearchEngine for MyEngine {
    fn name(&self) -> &'static str { "myengine" }

    fn search<'a>(
        &'a self,
        query: &'a str,
        max_results: usize,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, EngineError>> {
        Box::pin(async move {
            // fetch HTML, parse with scraper, return Vec<SearchResult>
        })
    }
}
```

Add it to `engines/mod.rs` and wire it in `main.rs`. Give the struct a
`timeout: Duration` field and a `new()`/`with_timeout()` constructor pair like
the built-in engines, and pass the timeout into the module's `search` function.
