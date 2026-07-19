# BeeCrawl Node.js SDK

The BeeCrawl SDK is a thin Node.js client for the BeeCrawl HTTP API. It does
not embed a browser or scraper; rendering, caching, and workers stay on the
BeeCrawl server.

## Install

```bash
npm install beecrawl-sdk
```

For local development:

```bash
pnpm --filter beecrawl-sdk build
```

## Usage

```js
import { BeeCrawlClient } from "beecrawl-sdk";

const client = new BeeCrawlClient({
  apiKey: "your-key",
  baseUrl: "https://api.beecrawl.dev",
});

const page = await client.scrape("https://example.com", {
  formats: ["markdown", "html", "links"],
  use_browser: "auto",
});

const mapped = await client.map("https://example.com", { limit: 100 });
const extracted = await client.extract(
  "https://example.com",
  { title: "Page title", email: "Contact email" },
);
```

For local development, use `baseUrl: "http://127.0.0.1:8000"`.

Asynchronous jobs can be submitted and polled through the same client:

```js
const job = await client.crawl("https://example.com", {
  limit: 100,
  maxDepth: 2,
});
const result = await client.pollCrawl(job.id, { intervalMs: 2000 });

const batch = await client.batchScrape([
  "https://example.com",
  "https://example.org",
]);
const batchResult = await client.pollBatchScrape(batch.id);
```

The `v2*`, browser, Agent, and Monitor methods cover the complete public v2
surface, including document upload/reference flows, job errors and
cancellation, replay, scrape interaction handoff, monitor updates and checks.
