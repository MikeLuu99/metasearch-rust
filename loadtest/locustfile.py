# Load test with: uv run locust -f locustfile.py --config locust.conf
# See README.md -> "Load testing" for full instructions.
#
# NOTE: the server fans out every /search to DuckDuckGo, Brave, Startpage and
# Yahoo (and /images to Bing, Google, Sogou), so load hits those upstream
# engines too. To avoid getting the server's IP rate-limited or blocked:
#   * run against the response-cache build (CACHE_TTL_MS > 0) so repeated
#     queries are absorbed in memory instead of re-hitting engines;
#   * ENGINE_MAX_CONCURRENCY caps how many requests each engine sees at once;
#   * keep the user count modest when testing against real engines.
from __future__ import annotations

import random

from locust import HttpUser, between, tag, task

_QUERIES = [
    "rust programming",
    "tokio async runtime",
    "axum web framework",
    "serde json rust",
    "cargo workspace",
    "async await rust",
    "actix web vs axum",
    "rust borrow checker",
    "rust lifetime annotations",
    "rayon parallel iterators",
    "rust error handling",
    "clap command line arguments",
    "tower middleware",
    "reqwest http client",
    "rust web assembly",
    "sqlx database driver",
    "rust type system",
    "rust futures stream",
    "hyper http server",
    "rust procedural macros",
    "rust trait objects",
    "tokio select",
    "rust memory safety",
    "moka cache rust",
    "rust macros",
    "rust testing",
    "rust compiler",
    "web scraping rust",
    "html parsing rust",
    "rust regex",
    "http2 keep alive",
    "tcp socket programming",
    "rust async streams",
    "backend load testing",
    "api rate limiting",
    "reverse proxy nginx",
    "search engine ranking",
    "reciprocal rank fusion",
    "metadata extraction",
    "user agent headers",
]


class SearchUser(HttpUser):
    """Load profile: mostly web searches, some image searches, light health checks."""

    wait_time = between(1, 5)

    def _random_query(self) -> str:
        return random.choice(_QUERIES)

    @task(7)
    def search(self) -> None:
        with self.client.get(
            "/search", params={"q": self._random_query()}, catch_response=True
        ) as response:
            self._check_results(response)

    @task(2)
    def search_images(self) -> None:
        with self.client.get(
            "/images", params={"q": self._random_query()}, catch_response=True
        ) as response:
            self._check_results(response)

    @task(1)
    @tag("health")
    def health(self) -> None:
        with self.client.get("/health", catch_response=True) as response:
            if response.status_code != 200:
                response.failure(f"health returned {response.status_code}")

    @staticmethod
    def _check_results(response) -> None:
        if response.status_code != 200:
            response.failure(f"status {response.status_code}")
            return
        try:
            data = response.json()
        except ValueError:
            response.failure("response is not valid JSON")
            return
        if "results" not in data:
            response.failure("response missing 'results' field")
