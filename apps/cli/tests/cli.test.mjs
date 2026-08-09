import assert from "node:assert/strict";
import { mkdtemp, rm, stat } from "node:fs/promises";
import test from "node:test";
import { join } from "node:path";
import { tmpdir } from "node:os";

import { ConfigStore, defaultConfigPath, parseArgs, renderResult, runCli } from "../dist/index.js";

test("parses repeated flags and boolean flags", () => {
  const parsed = parseArgs(["search", "query", "--source", "web", "--source=news", "--json"]);
  assert.deepEqual(parsed.positionals, ["search", "query"]);
  assert.deepEqual(parsed.getAll("source"), ["web", "news"]);
  assert.equal(parsed.boolean("json"), true);
});

test("renders scrape markdown from a v2 document envelope", () => {
  assert.equal(renderResult({ success: true, data: { markdown: "# Hello" } }, "markdown"), "# Hello\n");
});

test("uses platform configuration paths", () => {
  assert.equal(defaultConfigPath("darwin", "/Users/test"), "/Users/test/Library/Application Support/beecrawl/config.json");
  assert.equal(defaultConfigPath("linux", "/home/test", { XDG_CONFIG_HOME: "/tmp/config" }), "/tmp/config/beecrawl/config.json");
});

test("profile commands never print the API key", async () => {
  const configPath = "/tmp/beecrawl-cli-test-config.json";
  const store = new ConfigStore(configPath);
  await store.setProfile("default", { apiUrl: "https://api.test", apiKey: "secret", source: "test" });
  const output = { stdout: "", stderr: "" };
  const code = await runCli(["profile", "current"], {
    stdout: { write: (value) => { output.stdout += value; } },
    stderr: { write: (value) => { output.stderr += value; } },
  }, { configPath });
  assert.equal(code, 0);
  assert.equal(output.stdout.includes("secret"), false);
  assert.equal(output.stdout.includes("default"), true);
});

test("maps API failures to stable auth exit code and JSON stderr", async () => {
  const output = { stdout: "", stderr: "" };
  const code = await runCli(["scrape", "https://example.com", "--json"], {
    stdout: { write: (value) => { output.stdout += value; } },
    stderr: { write: (value) => { output.stderr += value; } },
  }, { env: {} });
  assert.equal(code, 3);
  assert.equal(JSON.parse(output.stderr).code, 3);
});

test("maps search flags to the v2 payload and keeps stdout machine-readable", async () => {
  const calls = [];
  const output = { stdout: "", stderr: "" };
  const code = await runCli(["search", "web scraping", "--limit", "20", "--source", "web", "--source", "news", "--include-domain", "example.com", "--json"], {
    stdout: { write: (value) => { output.stdout += value; } },
    stderr: { write: (value) => { output.stderr += value; } },
  }, {
    env: { BEECRAWL_API_KEY: "test-key" },
    clientFactory: () => ({
      v2Search: async (query, options) => { calls.push({ query, options }); return { results: [] }; },
      v2Scrape: async () => ({}), v2Map: async () => ({}), v2Extract: async () => ({}), v2Crawl: async () => ({}),
      v2JobStatus: async () => ({}), cancelV2Job: async () => ({}), createAgent: async () => ({}), getAgent: async () => ({}), cancelAgent: async () => ({}),
    }),
  });
  assert.equal(code, 0);
  assert.deepEqual(calls, [{ query: "web scraping", options: { limit: 20, sources: ["web", "news"], includeDomains: ["example.com"] } }]);
  assert.deepEqual(JSON.parse(output.stdout), { results: [] });
  assert.equal(output.stderr, "");
});

test("crawl start submits a v2 job without polling", async () => {
  let polled = false;
  const output = { stdout: "", stderr: "" };
  const code = await runCli(["crawl", "start", "https://example.com", "--max-depth", "2", "--no-wait", "--json"], {
    stdout: { write: (value) => { output.stdout += value; } },
    stderr: { write: (value) => { output.stderr += value; } },
  }, {
    env: { BEECRAWL_API_KEY: "test-key" },
    clientFactory: () => ({
      v2Search: async () => ({}), v2Scrape: async () => ({}), v2Map: async () => ({}), v2Extract: async () => ({}),
      v2Crawl: async (_url, options) => ({ id: "crawl-1", status: "queued", options }),
      v2JobStatus: async () => { polled = true; return { id: "crawl-1", status: "completed" }; }, cancelV2Job: async () => ({}),
      createAgent: async () => ({}), getAgent: async () => ({}), cancelAgent: async () => ({}),
    }),
  });
  assert.equal(code, 0);
  assert.equal(polled, false);
  assert.deepEqual(JSON.parse(output.stdout), { id: "crawl-1", status: "queued", options: { maxDiscoveryDepth: 2 } });
});

test("login exchanges a loopback callback and saves the returned profile", async () => {
  const directory = await mkdtemp(join(tmpdir(), "beecrawl-cli-login-"));
  const output = { stdout: "", stderr: "" };
  try {
    const code = await runCli(["login", "--profile", "staging", "--dashboard-url", "https://dashboard.test"], {
      stdout: { write: (value) => { output.stdout += value; } },
      stderr: { write: (value) => { output.stderr += value; } },
    }, {
      configPath: join(directory, "config.json"),
      openBrowser: async (url) => {
        const authorization = new URL(url);
        const callback = new URL(authorization.searchParams.get("redirect_uri"));
        setTimeout(() => { void fetch(`${callback}?code=one-time-code&state=${authorization.searchParams.get("state")}`); }, 0);
      },
      fetch: async (url) => {
        assert.equal(url, "https://dashboard.test/api/v1/cli/token");
        return new Response(JSON.stringify({ apiKey: "bcr_secret", apiUrl: "https://api.test", workspaceId: "workspace-1" }), { status: 200, headers: { "Content-Type": "application/json" } });
      },
    });
    assert.equal(code, 0);
    assert.deepEqual(JSON.parse(output.stdout), { profile: "staging", apiUrl: "https://api.test", workspaceId: "workspace-1" });
    const profile = await new ConfigStore(join(directory, "config.json")).load();
    assert.equal(profile.profiles.staging.apiKey, "bcr_secret");
    assert.equal(profile.currentProfile, "staging");
    assert.equal((await stat(join(directory, "config.json"))).mode & 0o777, 0o600);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
