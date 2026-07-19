# BeeCrawl Python SDK

The BeeCrawl SDK is a thin Python client for the BeeCrawl HTTP API. It does
not embed a browser or scraper; rendering, caching, and workers stay on the
BeeCrawl server.

## Install

```bash
uv add beecrawl-py
```

For local development:

```bash
uv pip install -e apps/sdk/python
```

## Usage

```python
from beecrawl_sdk import BeeCrawlClient

client = BeeCrawlClient(
    api_key="your-key",
    base_url="https://api.beecrawl.dev",
)
page = client.scrape(
    "https://example.com",
    formats=["markdown", "html", "links"],
    use_browser="auto",
)
mapped = client.map("https://example.com", limit=100)
extracted = client.extract(
    "https://example.com",
    {"title": "Page title", "email": "Contact email"},
)
```

For local development, use `base_url="http://127.0.0.1:8000"`.

Asynchronous jobs can be submitted and polled through the same client:

```python
job = client.crawl("https://example.com", limit=100, maxDepth=2)
result = client.poll_crawl(job["id"], interval=2)

batch = client.batch_scrape(["https://example.com", "https://example.org"])
result = client.poll_batch_scrape(batch["id"])
```

An async client with the same API is available as `AsyncBeeCrawlClient`.

The `v2_*`, browser, Agent, and Monitor methods cover the complete public v2
surface, including document upload/reference flows, job errors and
cancellation, replay, scrape interaction handoff, monitor updates and checks.
Request option dictionaries are passed through so server additions remain
forward-compatible.
