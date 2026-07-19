# Firecrawl Capability Gap TODO

This checklist tracks the implementation work required to make BeeCrawl a
practical, self-hosted replacement for the Firecrawl v2 API. Items are ordered
by dependency and user impact. A checkbox is complete only when the API,
documentation, and automated tests all cover the behavior.

Reference baseline: Firecrawl `cf5045315` (2026-07-17).

## 1. Async job protocol compatibility

- [x] Add `POST /v2/batch/scrape`.
- [x] Add `GET /v2/batch/scrape/{id}` with Firecrawl pagination fields.
- [x] Add `DELETE /v2/batch/scrape/{id}`.
- [x] Add crawl and batch `/{id}/errors` endpoints.
- [x] Add `/v2/crawl/active` and `/v2/crawl/ongoing`.
- [x] Return stable job IDs, expiry metadata, pagination URLs, and error shapes.

## 2. Current Firecrawl SDK contract

- [x] Pin a supported `firecrawl-py` version in the contract environment.
- [x] Cover scrape, map, crawl, batch, search, parse, and error responses.
- [x] Align current v2 defaults for map, crawl, search, and cache behavior.
- [x] Return map link objects (`url`, optional `title` and `description`).
- [x] Document intentional incompatibilities and reject unsupported semantics.

## 3. Scrape formats and content controls

- [x] Add `json` format using the existing OpenAI-compatible extraction layer.
- [x] Add `images` and `summary` formats.
- [x] Add screenshot `fullPage`, `quality`, and viewport options.
- [x] Add caller-supplied HTTP headers.
- [x] Add `includeTags`, `excludeTags`, `onlyMainContent`, and clean-content control.
- [x] Add structured `attributes`, `question`, and `highlights` formats.
- [x] Add deterministic JSON extraction.

## 4. Crawl policy and delivery

- [x] Respect robots.txt by default and support `robotsUserAgent`.
- [x] Add `includePaths`, `excludePaths`, and full-URL regex matching.
- [x] Implement all sitemap modes and external/subdomain/domain policies.
- [x] Add crawl delay and per-job maximum concurrency.
- [x] Add similar-URL deduplication and query-parameter policy parity.
- [x] Add idempotency keys.
- [x] Add signed webhooks for crawl and batch lifecycle events.
- [x] Add WebSocket job progress and document events.

## 5. Fetch success rate and engine selection

- [x] Add configurable basic proxy support.
- [x] Add enhanced/stealth browser modes and proxy selection.
- [x] Add TLS/browser fingerprint-aware HTTP fetching.
- [x] Add engine outcome metadata and fallback reasons.
- [x] Add content quality scoring and multi-engine selection.
- [x] Add distributed Bee Engine capacity and health reporting.

## 6. Public browser actions

- [x] Expose wait-by-duration and wait-by-selector.
- [x] Expose click, write, press, scroll, screenshot, and page scrape actions.
- [x] Expose JavaScript execution and PDF generation.
- [x] Return ordered action results and intermediate scrape documents.
- [x] Enforce action count, timeout, and payload limits.

## 7. File parsing

- [x] Add OCR for scanned PDFs.
- [x] Parse HTML uploads.
- [x] Parse DOC/DOCX, ODT, and RTF documents.
- [x] Parse XLS/XLSX workbooks.
- [x] Add upload-reference and pre-signed upload flows.
- [x] Support applicable scrape formats for parsed documents.

## 8. Search providers and filters

- [x] Add news and image search providers.
- [x] Add GitHub, research-paper, and PDF categories.
- [x] Add include/exclude domain filters.
- [x] Add time, language, country, and location controls.
- [x] Add asynchronous result scraping and query highlights.

## 9. Persistent browser sessions

- [x] Add create, list, execute, and delete session APIs.
- [x] Persist cookies and browser state for the session lifetime.
- [x] Add scrape-to-interactive-browser handoff.
- [x] Add session replay metadata and page snapshots.
- [x] Add session expiry, ownership, concurrency, and cleanup.

## 10. Higher-level workflows

- [x] Add asynchronous Agent jobs with status, cancellation, sources, and budgets.
- [x] Add scheduled Monitor jobs and manual runs.
- [x] Store monitor snapshots and expose check history.
- [x] Add JSON and text/git-style change tracking.
- [x] Add monitor notification webhooks.

## Cross-cutting requirements

- [x] Maintain SSRF protection, including DNS resolution to private networks.
- [x] Add per-key rate limits and concurrency controls suitable for self-hosting.
- [x] Add metrics for API latency, engine selection, queues, and failures.
- [x] Keep Python, Node.js, and Rust SDKs aligned with every public endpoint.
- [x] Keep OpenAPI and self-hosting documentation current.
