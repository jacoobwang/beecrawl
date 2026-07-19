import assert from "node:assert/strict";
import test from "node:test";

import { BeeCrawlClient, BeeCrawlError } from "../dist/index.js";

test("client requires baseUrl", () => {
  assert.throws(() => new BeeCrawlClient({ baseUrl: "" }), /baseUrl is required/);
});

test("client sends auth and scrape options", async () => {
  const requests = [];
  const client = new BeeCrawlClient({
    apiKey: "secret",
    baseUrl: "http://api.test/",
    fetch: async (url, init) => {
      requests.push({ url, init });
      return jsonResponse({ markdown: "hello" });
    },
  });

  const response = await client.scrape("https://example.com", {
    formats: ["markdown", "links"],
  });

  assert.deepEqual(response, { markdown: "hello" });
  assert.equal(requests[0].url, "http://api.test/scrape");
  assert.equal(requests[0].init.method, "POST");
  assert.equal(requests[0].init.headers.get("x-web-extract-api-key"), "secret");
  assert.deepEqual(JSON.parse(requests[0].init.body), {
    url: "https://example.com",
    formats: ["markdown", "links"],
  });
});

test("client parses api errors", async () => {
  const client = new BeeCrawlClient({
    baseUrl: "http://api.test",
    fetch: async () => jsonResponse(
      { detail: { code: "unauthorized", message: "Invalid key" } },
      { status: 401 },
    ),
  });

  await assert.rejects(
    () => client.map("https://example.com"),
    (error) => {
      assert.ok(error instanceof BeeCrawlError);
      assert.equal(error.statusCode, 401);
      assert.equal(error.detail.code, "unauthorized");
      assert.equal(error.message, "Invalid key");
      return true;
    },
  );
});

test("pollCrawl waits until a terminal state", async () => {
  const statuses = [
    { id: "job-1", status: "running" },
    { id: "job-1", status: "completed", data: [] },
  ];
  const client = new BeeCrawlClient({
    baseUrl: "http://api.test",
    fetch: async () => jsonResponse(statuses.shift()),
  });

  const result = await client.pollCrawl("job-1", {
    intervalMs: 0,
    timeoutMs: 100,
  });

  assert.equal(result.status, "completed");
});

test("v2 workflow and browser methods use public routes", async () => {
  const requests = [];
  const client = new BeeCrawlClient({
    baseUrl: "http://api.test",
    fetch: async (url, init) => {
      requests.push([init.method, new URL(url).pathname]);
      return jsonResponse({ success: true });
    },
  });
  await client.v2Scrape("https://example.com");
  await client.createBrowserSession();
  await client.executeBrowser("session", "document.title");
  await client.createAgent("research", { maxCredits: 2 });
  await client.createMonitor({ name: "site", url: "https://example.com" });
  await client.updateMonitor("monitor", { enabled: false });
  await client.runMonitor("monitor");
  await client.monitorChecks("monitor", "check");
  assert.deepEqual(requests, [
    ["POST", "/v2/scrape"],
    ["POST", "/v2/browser"],
    ["POST", "/v2/browser/session/execute"],
    ["POST", "/v2/agent"],
    ["POST", "/v2/monitor"],
    ["PATCH", "/v2/monitor/monitor"],
    ["POST", "/v2/monitor/monitor/run"],
    ["GET", "/v2/monitor/monitor/checks/check"],
  ]);
});

function jsonResponse(payload, init = {}) {
  return new Response(JSON.stringify(payload), {
    status: init.status ?? 200,
    headers: { "Content-Type": "application/json" },
  });
}
