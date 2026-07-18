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
- [ ] Add idempotency keys.
- [ ] Add signed webhooks for crawl and batch lifecycle events.
- [ ] Add WebSocket job progress and document events.

## 5. Fetch success rate and engine selection

- [ ] Add configurable basic proxy support.
- [ ] Add enhanced/stealth browser modes and proxy selection.
- [ ] Add TLS/browser fingerprint-aware HTTP fetching.
- [ ] Add engine outcome metadata and fallback reasons.
- [ ] Add content quality scoring and multi-engine selection.
- [ ] Add distributed Bee Engine capacity and health reporting.

## 6. Public browser actions

- [ ] Expose wait-by-duration and wait-by-selector.
- [ ] Expose click, write, press, scroll, screenshot, and page scrape actions.
- [ ] Expose JavaScript execution and PDF generation.
- [ ] Return ordered action results and intermediate scrape documents.
- [ ] Enforce action count, timeout, and payload limits.

## 7. File parsing

- [ ] Add OCR for scanned PDFs.
- [ ] Parse HTML uploads.
- [ ] Parse DOC/DOCX, ODT, and RTF documents.
- [ ] Parse XLS/XLSX workbooks.
- [ ] Add upload-reference and pre-signed upload flows.
- [ ] Support applicable scrape formats for parsed documents.

## 8. Search providers and filters

- [ ] Add news and image search providers.
- [ ] Add GitHub, research-paper, and PDF categories.
- [ ] Add include/exclude domain filters.
- [ ] Add time, language, country, and location controls.
- [ ] Add asynchronous result scraping and query highlights.

## 9. Persistent browser sessions

- [ ] Add create, list, execute, and delete session APIs.
- [ ] Persist cookies and browser state for the session lifetime.
- [ ] Add scrape-to-interactive-browser handoff.
- [ ] Add session replay metadata and page snapshots.
- [ ] Add session expiry, ownership, concurrency, and cleanup.

## 10. Higher-level workflows

- [ ] Add asynchronous Agent jobs with status, cancellation, sources, and budgets.
- [ ] Add scheduled Monitor jobs and manual runs.
- [ ] Store monitor snapshots and expose check history.
- [ ] Add JSON and text/git-style change tracking.
- [ ] Add monitor notification webhooks.

## Cross-cutting requirements

- [ ] Maintain SSRF protection, including DNS resolution to private networks.
- [ ] Add per-key rate limits and concurrency controls suitable for self-hosting.
- [ ] Add metrics for API latency, engine selection, queues, and failures.
- [ ] Keep Python, Node.js, and Rust SDKs aligned with every public endpoint.
- [ ] Keep OpenAPI and self-hosting documentation current.
