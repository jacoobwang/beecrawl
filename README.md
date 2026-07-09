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

### `POST /v1/scrape`

```json
{
  "url": "https://example.com"
}
```

Returns page title, markdown-like text, links, and metadata.

### `POST /v1/extract`

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
uvicorn beecrawl.app:app --reload
```

Then open:

```bash
curl -X POST http://127.0.0.1:8000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com"}'
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
