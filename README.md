# 🐝 BeeCrawl

BeeCrawl is an open-source Firecrawl alternative for teams that want to
self-host web scraping, crawling, search, and structured extraction.

**It provides a Firecrawl-style API surface with clean Markdown extraction,
browser-rendered scraping, URL discovery, keyword search, and deterministic
schema extraction**. BeeCrawl is designed to stay small and hackable while leaving
clear extension points for queue-backed crawls, LLM extraction,
source-specific providers, proxy infrastructure, and hosted deployments.

The API service is implemented in Rust. Browser rendering lives in the Python
Bee Engine service because Playwright's Python runtime is still the friendlier
browser automation boundary for this project.

## Goals

- Crawl web pages and return clean, useful content.
- Extract page metadata, readable text, links, and structured fields.
- Provide a small API surface that is easy to self-host.
- Keep provider integrations modular so teams can choose their own browser,
  proxy, storage, and LLM stack.

## API Preview

### `POST /scrape`

Firecrawl-style Markdown extraction endpoint migrated from
`workus-realtime-dataservice`:

```json
{
  "url": "https://example.com",
  "formats": ["markdown", "html", "rawHtml", "links", "screenshot"],
  "timeout_seconds": 30,
  "wait_for_ms": 0,
  "use_browser": "auto"
}
```

Returns `request_id`, `final_url`, `markdown`, and provider metadata. Request
`html` for the selected content root HTML, `rawHtml` for the complete fetched
or browser-rendered HTML, `links` for deduplicated absolute page links, or
`screenshot` for a PNG data URL. Screenshots require browser rendering. Set
`BEECRAWL_WEB_EXTRACT_API_KEY` or `WEB_EXTRACT_API_KEY` to require
`X-Web-Extract-Api-Key`, `X-Api-Key`, or bearer-token auth.

### `POST /map`

```json
{
  "url": "https://example.com",
  "limit": 100,
  "include_subdomains": false
}
```

Discovers same-site URLs from sitemap first, then page links.

### `POST /batch/scrape`

```json
{
  "urls": [
    "https://example.com",
    "https://example.com/docs"
  ],
  "use_browser": "auto",
  "maxRetries": 2
}
```

Creates one asynchronous job for multiple independent URLs. Duplicate URLs
are removed before enqueueing. Poll `GET /batch/scrape/{id}?offset=0&limit=20`
for the same paginated result shape as crawl, or use `DELETE
/batch/scrape/{id}` to cancel it. Batch scrape never follows links from the
submitted pages.

### `POST /crawl`

```json
{
  "url": "https://example.com",
  "limit": 100,
  "maxDepth": 2,
  "useBrowser": "auto"
}
```

Starts an asynchronous, same-site crawl. Poll `GET /crawl/{id}?offset=0&limit=20`
for progress and a page of collected results, or use `DELETE /crawl/{id}` to
request cancellation. `maxRetries` controls retry attempts after the first
failed scrape; it defaults to `2`. Jobs and results are stored in Postgres and
consumed by a separate worker process.

### `POST /search`

```json
{
  "query": "thermal interface material suppliers",
  "limit": 5,
  "scrapeOptions": {
    "formats": ["markdown"],
    "use_browser": "auto"
  }
}
```

Searches the web by keyword and returns result URLs, titles, and descriptions.
When `scrapeOptions.formats` is non-empty, BeeCrawl scrapes each result URL
with the existing scrape service and merges Markdown into the search results.

Set `BEECRAWL_SEARXNG_ENDPOINT` to use a self-hosted SearXNG instance. Without
SearXNG, BeeCrawl falls back to DuckDuckGo HTML search.

### `POST /extract`

```json
{
  "url": "https://example.com",
  "schema": {
    "company": "Company name",
    "email": "Contact email"
  },
  "use_browser": "auto"
}
```

Returns a structured JSON object. By default it uses deterministic page parsing.
Configure an OpenAI-compatible LLM provider to enable model-backed extraction:

```bash
BEECRAWL_LLM_PROVIDER=openai-compatible
BEECRAWL_LLM_API_KEY=...
BEECRAWL_LLM_BASE_URL=https://api.openai.com/v1
BEECRAWL_LLM_MODEL=gpt-4o-mini
```

Per-request provider overrides are also supported with `provider` or `llm`:

```json
{
  "url": "https://example.com",
  "schema": {
    "company": "Company name"
  },
  "provider": {
    "provider": "openai-compatible",
    "base_url": "https://dashscope.aliyuncs.com/compatible-mode/v1",
    "model": "qwen-plus"
  }
}
```

## Quick Start

Start the Rust API:

```bash
make api
```

For distributed crawls, start Postgres, configure `BEECRAWL_DATABASE_URL`, run
migrations, then start the API and worker separately. BeeCrawl uses `sqlx-cli`
for migration creation and execution.

```bash
make db-up
export BEECRAWL_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:55432/beecrawl
cargo install sqlx-cli --no-default-features --features postgres,rustls
make migrate-up
make api
```

In another terminal:

```bash
make worker
```

Crawl jobs are retained for seven days by default. The worker runs cleanup
hourly; `make crawl-cleanup` is also available for a scheduled job.

Browser rendering for `use_browser: "auto"` is provided by the Python Bee
Engine service:

```bash
make install
make playwright-install
make bee-engine
```

Browser rendering runs in Bee Engine. It reuses a Chromium instance and creates
an isolated context per request. Set `BEE_ENGINE_MAX_PAGES` to control
concurrent rendered pages; the default is `4`.

It exposes Fire Engine-style endpoints on port `8020` by default:

```text
POST   /scrape
GET    /scrape/{job_id}
DELETE /scrape/{job_id}
```

Then open:

```bash
curl -X POST http://127.0.0.1:8000/scrape \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com"}'
```

## Repository Layout

```text
apps/api      Rust API package
apps/bee-engine  Browser rendering service
apps/*-sdk    SDK packages
```

## Roadmap

- HTTP static scraper
- HTML to markdown-like text cleanup
- Link and metadata extraction
- Browser-rendered fallback
- Keyword search with optional result scraping
- Async crawl jobs
- JSON schema extraction
- Provider plugins
- Docker image
- Hosted cloud API

## License

MIT
