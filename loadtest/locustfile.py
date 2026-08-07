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
from pathlib import Path

from locust import HttpUser, between, tag, task

_QUERY_FILE = Path(__file__).with_name("queries.txt")

_FALLBACK_QUERIES = [
    "rust programming",
    "tokio async",
    "python",
    "javascript",
]


def _load_queries() -> list[str]:
    if not _QUERY_FILE.exists():
        return _FALLBACK_QUERIES
    return [
        line.strip()
        for line in _QUERY_FILE.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    ]


class SearchUser(HttpUser):
    """Load profile: mostly web searches, some image searches, light health checks."""

    wait_time = between(1, 5)

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.queries = _load_queries()

    def _random_query(self) -> str:
        return random.choice(self.queries)

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
