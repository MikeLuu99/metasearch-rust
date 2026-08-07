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
    "rust",
    "pasta",
    "volcanoes",
    "kiteboarding",
    "rust programming",
    "tokio async runtime",
    "best coffee brewing methods",
    "james webb telescope images",
    "how to make sourdough bread at home",
    "rust",
    "topology",
    "sushi",
    "quokkas",
    "urban gardening",
    "how neural networks learn",
    "electric car battery recycling",
    "best hiking trails in california",
    "history of the olympic games",
    "low carb dinner ideas for busy weeknights",
    "quantum",
    "jazz",
    "noodles",
    "astronomy",
    "marathon training",
    "sustainable fashion brands",
    "mediterranean diet meal plan",
    "machine learning for beginners",
    "things to do in tokyo on a budget",
    "how to fix a leaky kitchen faucet step by step",
    "vim",
    "yoga",
    "aquariums",
    "photography",
    "wildlife conservation",
    "diy woodworking projects",
    "renewable energy sources",
    "best books to read this year",
    "examples of renewable energy used in developing countries",
    "how to reduce plastic waste in everyday life tips",
    "crypto",
    "baking",
    "beekeeping",
    "ancient greece",
    "space exploration",
    "small business accounting",
    "benefits of meditation for anxiety",
    "beginner guide to learning a new language fast",
    "rust",
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
