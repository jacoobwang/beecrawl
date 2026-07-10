# Browser Rendering

BeeCrawl uses Playwright rendering for automatic scrapes, with plain HTTP fetch
as the fallback path when the browser path is unavailable or produces no
content. The current design follows the shape of Firecrawl's Playwright
microservice: the Rust API calls the Python Bee Engine service over HTTP.

## Request Flow

`POST /scrape` accepts:

```json
{
  "url": "https://example.com",
  "use_browser": "auto",
  "wait_for_ms": 0,
  "timeout_seconds": 30
}
```

`use_browser` controls the rendering path:

- `never`: fetch with plain HTTP only.
- `always`: render with Playwright only.
- `auto`: render with Playwright first. If Playwright fails or produces empty
  Markdown, fall back to plain HTTP fetch.

There is no Markdown length threshold in the `auto` path. The browser result is
accepted when it produces non-empty Markdown; otherwise BeeCrawl falls back to
fetch and lets the normal empty-content handling decide the final response.

## Browser Pool

Browser rendering is implemented in
`apps/bee-engine/bee_engine/browser.py`.

Bee Engine keeps one Chromium browser instance alive per process. Each rendered
request creates a fresh browser context and page:

1. Lazily start Playwright and launch Chromium on the first rendered request.
2. Acquire a semaphore permit.
3. Create a new context with a desktop viewport and stable user agent.
4. Block service workers, media, fonts, images, and common ad/analytics hosts.
5. Create a new page and navigate with `domcontentloaded`.
6. Best-effort wait for `networkidle` for up to 5 seconds.
7. Apply `wait_for_ms` if provided.
8. Read `page.content()`.
9. Close the page and context.
10. Release the semaphore permit.

This avoids the high cost of launching Chromium for every scrape while keeping
per-request cookies, storage, and page state isolated.

## Configuration

`BEE_ENGINE_MAX_PAGES` controls concurrent rendered pages per Bee Engine process.
The default is `4`. The Rust API uses `BEE_ENGINE_URL` to locate Bee Engine; the
default is `http://127.0.0.1:8020`.

Browser rendering requires the optional dependency and Chromium browser binary:

```bash
uv pip install -e ".[browser]"
.venv/bin/playwright install chromium
```

## Differences From Firecrawl

Firecrawl's Playwright implementation runs as a separate microservice. It keeps
a global browser instance, creates a new context per request, uses a semaphore
for concurrency, and closes the context after each scrape.

BeeCrawl uses the same lifecycle model in Bee Engine. The API service is Rust;
the browser service stays Python so Playwright remains isolated behind an engine
boundary.

Firecrawl also has additional engines such as index, fire-engine, TLS client,
and stealth proxy. BeeCrawl does not implement those yet.

## Known Limitations

- Browser rendering requires running Bee Engine for browser-first scrapes.
- A browser crash affects the current Bee Engine worker process until the pool
  relaunches Chromium.
- There is no distributed browser capacity across Bee Engine processes.
- There is no proxy, stealth, persistent profile, action execution, screenshot,
  or selector-wait API yet.
- The automatic path does not yet compare multiple successful engines or score
  Markdown quality.

## Future Work

- Add structured fallback metadata, such as `fallback_reason`.
- Add optional quality scoring if multiple successful engines are available.
- Add optional `check_selector` support for pages that need a specific element.
- Add explicit browser pool health and shutdown hooks.
- Add Rust API fallback metadata that reports when Bee Engine was unavailable.
