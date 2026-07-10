# Bee Engine

Bee Engine is the browser rendering service for Beecrawl. It provides a
Fire Engine-style API boundary while keeping the first implementation focused on
Playwright.

## Run Locally

```bash
uv pip install -e ".[browser]"
.venv/bin/playwright install chromium
make bee-engine
```

The service listens on `127.0.0.1:8020` by default.

## API

`POST /scrape` accepts:

```json
{
  "url": "https://example.com",
  "engine": "playwright",
  "instantReturn": false,
  "headers": {},
  "actions": [
    { "type": "wait", "milliseconds": 1000 },
    { "type": "screenshot", "fullPage": true },
    { "type": "executeJavascript", "script": "document.title" },
    { "type": "scrape" },
    { "type": "getCookies" }
  ],
  "timeout": 300000,
  "mobile": false,
  "blockMedia": true
}
```

When `instantReturn` is false, the response contains rendered HTML and metadata:

```json
{
  "timeTaken": 1234,
  "content": "<html>...</html>",
  "url": "https://example.com/",
  "pageStatusCode": 200,
  "responseHeaders": {},
  "screenshots": [],
  "actionContent": [],
  "actionResults": [],
  "usedMobileProxy": false
}
```

When `instantReturn` is true, Bee Engine stores the job in memory and returns:

```json
{
  "jobId": "job-id",
  "processing": true
}
```

Use `GET /scrape/{job_id}` to read the status/result and
`DELETE /scrape/{job_id}` to remove it.

## Implementation

The service keeps one Chromium browser instance per process. Each request gets a
fresh isolated browser context and page. Media, images, fonts, and common
analytics hosts are blocked by default when `blockMedia` is true.

The initial implementation uses Playwright APIs. The `chrome-cdp` engine value is
accepted as a compatibility alias, but it currently follows the same Playwright
path. CDP-specific behavior can be added behind the same endpoint later.
