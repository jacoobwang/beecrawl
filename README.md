# BeeCrawl

Open-source infrastructure for crawling, extracting, and structuring web data.

BeeCrawl is a developer-first web data pipeline. It starts with simple HTTP
scraping and structured extraction, then leaves clear extension points for
browser rendering, queue-backed crawls, LLM extraction, and source-specific
providers.

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
  "formats": ["markdown"],
  "timeout_seconds": 30,
  "use_browser": "auto"
}
```

Returns `request_id`, `final_url`, `markdown`, and provider metadata. Set
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

### `POST /extract`

```json
{
  "url": "https://example.com",
  "schema": {
    "company": "Company name",
    "email": "Contact email"
  }
}
```

Returns a structured JSON object. The initial implementation uses deterministic
page parsing; an LLM-backed extractor can be added behind the same contract.

## Quick Start

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
make api
```

Browser-rendered fallback is optional:

```bash
pip install -e ".[browser]"
playwright install chromium
```

Then open:

```bash
curl -X POST http://127.0.0.1:8000/scrape \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com"}'
```

## Repository Layout

```text
apps/api      API and worker package
apps/*-sdk    SDK packages
```

## Roadmap

- HTTP static scraper
- HTML to markdown-like text cleanup
- Link and metadata extraction
- Browser-rendered fallback
- Async crawl jobs
- JSON schema extraction
- Provider plugins
- Docker image
- Hosted cloud API

## License

Apache-2.0
