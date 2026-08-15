# 🐝 BeeCrawl

[简体中文](README-ZH.md)

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

Scrape caching is enabled by default when Postgres is configured. The request
path is `cache -> browser -> fetch`; cache reads fail open, and formats are
derived from the cached HTML snapshot.

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

### Firecrawl v2 compatibility

The API also exposes Firecrawl v2-compatible routes for applications using the
pinned `firecrawl-py==4.32.1` contract:

```text
POST   /v2/scrape
POST   /v2/parse
POST   /v2/parse/base64
POST   /v2/map
POST   /v2/crawl
GET    /v2/crawl/active
GET    /v2/crawl/ongoing
GET    /v2/crawl/{id}
DELETE /v2/crawl/{id}
GET    /v2/crawl/{id}/errors
POST   /v2/batch/scrape
GET    /v2/batch/scrape/{id}
DELETE /v2/batch/scrape/{id}
GET    /v2/batch/scrape/{id}/errors
POST   /v2/extract
POST   /v2/search
```

Set the Firecrawl SDK `api_url` to the BeeCrawl base URL. These routes accept
Firecrawl camelCase request fields and return its `success` response envelope.
Unsupported fields, format-specific options, and behavior-changing option
values return JSON `400` responses instead of being silently ignored. The
default scrape options emitted by `firecrawl-py` 4.32.1 are accepted, including
working `skipTlsVerification` support. Run `make firecrawl-contract` against a
local API to verify the adapter through the official Python SDK.
The v2 extract adapter supports multiple URLs and JSON Schema objects. Search
supports Web results with optional scraping; requested news and image groups
are returned empty until providers for those source types are added. Batch
scrape, error listing, active crawl discovery, and paginated job status are
part of the compatibility surface. Usage-account endpoints are not implemented.

`POST /v2/parse` accepts a local PDF as `multipart/form-data`: a required
`file` field and an optional JSON `options` field. It returns Markdown with
`metadata.numPages`, `metadata.totalPages`, and `metadata.sourceFile`. The
current parser supports text PDFs in `fast` or `auto` mode; OCR and non-PDF
document formats are intentionally rejected.

For JSON-only callers, `POST /v2/parse/base64` accepts `base64` (or `data`),
`filename`, and optional `options`. It accepts either bare Base64 or a
`data:application/pdf;base64,...` value; decoded PDFs remain limited to 50 MB.

## Scrape quality evaluations

Deterministic tests and live scrape quality evaluations are separate. With the
API and Bee Engine running locally, execute:

```bash
make scrape-eval
```

The evaluation suite exercises static, document-heavy, and JavaScript-rendered
pages, compares observable output against checked-in quality gates, and writes
JSON and Markdown reports. See [Scrape quality evaluations](docs/scrape-evals.md)
for case authoring and the `#scrape-quality-eval` pull request workflow.

For a reproducible comparison against other providers, run the separate
multi-sample benchmark. It reports success rate, content quality, p50/p95/p99
latency, errors, and successful pages per minute:

```bash
make scrape-benchmark
```

The benchmark suite can compare BeeCrawl with Firecrawl and Teracrawl. See
[Scrape quality evaluations](docs/scrape-evals.md) for provider configuration,
fresh-cache and warm-cache tracks, and raw report output.

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
hourly; scrape cache entries are reused for four hours by default and retained
for seven days. `make crawl-cleanup` is also available for a scheduled job.

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

### Python SDK

The HTTP-only Python SDK is available under `apps/sdk/python`:

```bash
uv pip install -e apps/sdk/python
```

It provides synchronous and asynchronous clients for `/scrape`, `/map`,
`/search`, `/extract`, `/crawl`, and `/batch/scrape`. The SDK does not run a
browser locally; browser rendering and workers stay on the BeeCrawl server.

### Node.js SDK

The Node.js SDK is available under `apps/sdk/node`:

```bash
npm install beecrawl-sdk
pnpm --filter beecrawl-sdk build
```

It provides a TypeScript client for `/scrape`, `/map`, `/search`, `/extract`,
`/crawl`, and `/batch/scrape` using Node 18+ native `fetch`.

```js
import { BeeCrawlClient } from "beecrawl-sdk";

const client = new BeeCrawlClient({
  apiKey: "your-key",
  baseUrl: "https://api.beecrawl.dev",
});

const page = await client.scrape("https://example.com", {
  formats: ["markdown", "links"],
});
```

### BeeCrawl CLI

The TypeScript CLI is available under `apps/cli` and requires Node.js 18 or
later. It uses the v2 API through the Node.js SDK:

```bash
pnpm install
pnpm cli:build
node apps/cli/dist/main.js --help
```

Authenticate with the Dashboard before making API requests. The CLI stores a
named local profile containing the authorized API URL and key:

```bash
node apps/cli/dist/main.js login
node apps/cli/dist/main.js profile current
```

For automation or local development, provide an API key through the environment
instead of saving a profile:

```bash
BEECRAWL_API_KEY=your-key \
BEECRAWL_BASE_URL=http://127.0.0.1:8000 \
node apps/cli/dist/main.js scrape https://example.com
```

The available data commands are `search`, `scrape`, `map`, `extract`, `crawl`,
and `agent`:

```bash
node apps/cli/dist/main.js search "web scraping" --limit 5
node apps/cli/dist/main.js scrape https://example.com
node apps/cli/dist/main.js map https://example.com --json
node apps/cli/dist/main.js extract https://example.com \
  --schema '{"title":"Page title"}' --json
node apps/cli/dist/main.js crawl https://example.com --no-wait --json
node apps/cli/dist/main.js agent "Summarize the main topics on this site" --json
```

`scrape` prints Markdown by default; the other data commands print JSON.
Use `--json` or `--format json` for machine-readable output, and
`--options-file` for nested API options. Crawl and Agent commands wait for a
terminal result by default; use `start`, `status`, `cancel`, or `--no-wait` for
detached workflows. `node apps/cli/dist/main.js init --agent codex` installs the
bundled Agent Skill, and `profile list|use|remove` manages local credential
profiles.

### Rust SDK

An asynchronous Rust SDK is available under `apps/sdk/rust` and can be added
as the published `beecrawl-sdk` Cargo dependency:

```toml
[dependencies]
beecrawl-sdk = "0.1"
```

Both SDKs are released together from an `sdk-v<version>` tag. The repository
publishes the Python package to PyPI, the npm package to npmjs.com, and the
Rust crate to crates.io. The release workflow verifies that all three package
versions match the tag and requires the `PYPI_API_TOKEN`, `NPM_TOKEN`, and
`CARGO_REGISTRY_TOKEN` repository secrets.

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
apps/sdk/node    Node.js SDK package
apps/sdk/python  Python SDK package
apps/sdk/rust    Rust SDK crate
apps/cli         Node.js CLI package
```

## Roadmap

### CLI and SDK ecosystem

- Publish `beecrawl-cli` as a versioned npm package with a release workflow,
  Dashboard authorization, and onboarding examples for Claude Code, Codex, and
  OpenCode.
- Extend the CLI to cover the remaining public v2 workflows: batch scraping,
  document parsing, browser sessions and interactions, and monitors.
- Keep the Python, Node.js, and Rust SDKs aligned with the complete public API
  and publish matching versions together.

### Scrape quality and operations

- Expand scrape-quality evaluations with code-heavy, malformed-HTML, PDF, and
  JavaScript-heavy fixtures, plus regression gates for Markdown and metadata.
- Add conversion and engine metrics such as input/output size, latency,
  empty-output rate, fallback reason, and provider selection.
- Improve Bee Engine lifecycle management with explicit health, shutdown, and
  capacity controls across multiple instances.

### Self-hosted platform

- Add pluggable Markdown conversion providers and configurable domain-specific
  content cleanup policies.
- Improve Docker Compose and Helm deployment ergonomics, including upgrades,
  secret configuration, and independent worker/browser scaling.
- Continue Firecrawl compatibility where it provides user value while keeping
  unsupported hosted usage-account behavior explicit.

## License

MIT
