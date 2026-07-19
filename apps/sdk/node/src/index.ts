export type JsonObject = Record<string, unknown>;

export interface BeeCrawlClientOptions {
  apiKey?: string;
  baseUrl: string;
  fetch?: typeof fetch;
}

export interface RequestOptions extends JsonObject {}

export interface PollOptions {
  offset?: number;
  limit?: number;
  intervalMs?: number;
  timeoutMs?: number;
}

export class BeeCrawlError extends Error {
  statusCode?: number;
  detail?: unknown;

  constructor(message: string, options: { statusCode?: number; detail?: unknown; cause?: unknown } = {}) {
    super(message, { cause: options.cause });
    this.name = "BeeCrawlError";
    this.statusCode = options.statusCode;
    this.detail = options.detail;
  }
}

export class BeeCrawlClient {
  readonly baseUrl: string;

  private readonly apiKey?: string;
  private readonly fetchImpl: typeof fetch;

  constructor(options: BeeCrawlClientOptions) {
    if (!options.baseUrl) {
      throw new BeeCrawlError("BeeCrawl baseUrl is required");
    }
    this.baseUrl = options.baseUrl.replace(/\/+$/, "");
    if (!this.baseUrl.startsWith("http://") && !this.baseUrl.startsWith("https://")) {
      throw new BeeCrawlError(`Invalid BeeCrawl baseUrl: ${this.baseUrl}`);
    }
    this.apiKey = options.apiKey;
    this.fetchImpl = options.fetch ?? globalThis.fetch;
    if (!this.fetchImpl) {
      throw new BeeCrawlError("A fetch implementation is required");
    }
  }

  scrape(url: string, options: RequestOptions = {}): Promise<JsonObject> {
    return this.post("/scrape", { ...options, url });
  }

  map(url: string, options: RequestOptions = {}): Promise<JsonObject> {
    return this.post("/map", { ...options, url });
  }

  search(query: string, options: RequestOptions = {}): Promise<JsonObject> {
    return this.post("/search", { ...options, query });
  }

  extract(url: string, schema: Record<string, string>, options: RequestOptions = {}): Promise<JsonObject> {
    return this.post("/extract", { ...options, url, schema });
  }

  crawl(url: string, options: RequestOptions = {}): Promise<JsonObject> {
    return this.post("/crawl", { ...options, url });
  }

  batchScrape(urls: string[], options: RequestOptions = {}): Promise<JsonObject> {
    return this.post("/batch/scrape", { ...options, urls });
  }

  crawlStatus(jobId: string, options: Pick<PollOptions, "offset" | "limit"> = {}): Promise<JsonObject> {
    return this.get(`/crawl/${jobId}`, {
      offset: options.offset ?? 0,
      limit: options.limit ?? 20,
    });
  }

  batchScrapeStatus(jobId: string, options: Pick<PollOptions, "offset" | "limit"> = {}): Promise<JsonObject> {
    return this.get(`/batch/scrape/${jobId}`, {
      offset: options.offset ?? 0,
      limit: options.limit ?? 20,
    });
  }

  cancelCrawl(jobId: string): Promise<JsonObject> {
    return this.delete(`/crawl/${jobId}`);
  }

  cancelBatchScrape(jobId: string): Promise<JsonObject> {
    return this.delete(`/batch/scrape/${jobId}`);
  }

  v2Scrape(url: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/scrape", { ...options, url }); }
  v2Map(url: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/map", { ...options, url }); }
  v2Search(query: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/search", { ...options, query }); }
  v2Extract(urls: string[], options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/extract", { ...options, urls }); }
  v2ParseBase64(base64: string, filename: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/parse/base64", { ...options, base64, filename }); }
  v2ParseReference(uploadRef: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/parse/reference", { ...options, uploadRef }); }
  createParseUpload(filename: string): Promise<JsonObject> { return this.post("/v2/parse/upload-url", { filename }); }
  uploadParseDocument(uploadRef: string, data: BodyInit): Promise<JsonObject> { return this.request("PUT", `/v2/parse/upload/${uploadRef}`, { body: data }); }
  v2Parse(filename: string, data: Blob, options: RequestOptions = {}): Promise<JsonObject> {
    const form = new FormData(); form.set("file", data, filename); form.set("options", JSON.stringify(options));
    return this.request("POST", "/v2/parse", { body: form });
  }
  v2Crawl(url: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/crawl", { ...options, url }); }
  v2BatchScrape(urls: string[], options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/batch/scrape", { ...options, urls }); }
  v2JobStatus(kind: "crawl" | "batch/scrape", jobId: string, params: Record<string, string | number> = {}): Promise<JsonObject> { return this.get(`/v2/${kind}/${jobId}`, params); }
  v2JobErrors(kind: "crawl" | "batch/scrape", jobId: string): Promise<JsonObject> { return this.get(`/v2/${kind}/${jobId}/errors`); }
  cancelV2Job(kind: "crawl" | "batch/scrape", jobId: string): Promise<JsonObject> { return this.delete(`/v2/${kind}/${jobId}`); }
  activeCrawls(): Promise<JsonObject> { return this.get("/v2/crawl/active"); }
  createBrowserSession(options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/browser", options); }
  browserSessions(): Promise<JsonObject> { return this.get("/v2/browser"); }
  executeBrowser(sessionId: string, code: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post(`/v2/browser/${sessionId}/execute`, { ...options, code }); }
  browserReplay(sessionId: string, pageId?: string): Promise<JsonObject> { return this.get(`/v2/browser/${sessionId}/replay${pageId ? `/${pageId}` : ""}`); }
  deleteBrowserSession(sessionId: string): Promise<JsonObject> { return this.delete(`/v2/browser/${sessionId}`); }
  interactWithScrape(scrapeId: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post(`/v2/scrape/${scrapeId}/interact`, options); }
  deleteScrapeInteraction(scrapeId: string): Promise<JsonObject> { return this.delete(`/v2/scrape/${scrapeId}/interact`); }
  createAgent(prompt: string, options: RequestOptions = {}): Promise<JsonObject> { return this.post("/v2/agent", { ...options, prompt }); }
  getAgent(jobId: string): Promise<JsonObject> { return this.get(`/v2/agent/${jobId}`); }
  cancelAgent(jobId: string): Promise<JsonObject> { return this.delete(`/v2/agent/${jobId}`); }
  createMonitor(payload: RequestOptions): Promise<JsonObject> { return this.post("/v2/monitor", payload); }
  listMonitors(): Promise<JsonObject> { return this.get("/v2/monitor"); }
  getMonitor(monitorId: string): Promise<JsonObject> { return this.get(`/v2/monitor/${monitorId}`); }
  updateMonitor(monitorId: string, payload: RequestOptions): Promise<JsonObject> { return this.patch(`/v2/monitor/${monitorId}`, payload); }
  deleteMonitor(monitorId: string): Promise<JsonObject> { return this.delete(`/v2/monitor/${monitorId}`); }
  runMonitor(monitorId: string): Promise<JsonObject> { return this.post(`/v2/monitor/${monitorId}/run`, {}); }
  monitorChecks(monitorId: string, checkId?: string): Promise<JsonObject> { return this.get(`/v2/monitor/${monitorId}/checks${checkId ? `/${checkId}` : ""}`); }

  pollCrawl(jobId: string, options: PollOptions = {}): Promise<JsonObject> {
    return this.poll(() => this.crawlStatus(jobId, options), jobId, options);
  }

  pollBatchScrape(jobId: string, options: PollOptions = {}): Promise<JsonObject> {
    return this.poll(() => this.batchScrapeStatus(jobId, options), jobId, options);
  }

  private async poll(status: () => Promise<JsonObject>, jobId: string, options: PollOptions): Promise<JsonObject> {
    const intervalMs = options.intervalMs ?? 1000;
    const timeoutMs = options.timeoutMs ?? 300_000;
    const deadline = Date.now() + timeoutMs;

    while (true) {
      const result = await status();
      if (["completed", "failed", "cancelled"].includes(String(result.status))) {
        return result;
      }
      if (Date.now() >= deadline) {
        throw new BeeCrawlError(`Timed out waiting for job ${jobId}`);
      }
      await sleep(intervalMs);
    }
  }

  private post(path: string, payload: JsonObject): Promise<JsonObject> {
    return this.request("POST", path, { body: JSON.stringify(payload) });
  }

  private patch(path: string, payload: JsonObject): Promise<JsonObject> {
    return this.request("PATCH", path, { body: JSON.stringify(payload) });
  }

  private get(path: string, params: Record<string, string | number> = {}): Promise<JsonObject> {
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(params)) {
      query.set(key, String(value));
    }
    return this.request("GET", `${path}?${query.toString()}`);
  }

  private delete(path: string): Promise<JsonObject> {
    return this.request("DELETE", path);
  }

  private async request(method: string, path: string, init: RequestInit = {}): Promise<JsonObject> {
    const headers = new Headers(init.headers);
    if (!(init.body instanceof FormData) && !headers.has("Content-Type")) {
      headers.set("Content-Type", "application/json");
    }
    if (this.apiKey) {
      headers.set("X-Web-Extract-Api-Key", this.apiKey);
    }

    let response: Response;
    try {
      response = await this.fetchImpl(`${this.baseUrl}${path}`, {
        ...init,
        method,
        headers,
      });
    } catch (error) {
      throw new BeeCrawlError(`BeeCrawl request failed: ${errorMessage(error)}`, { cause: error });
    }

    const payload = await parseJsonResponse(response);
    if (!response.ok) {
      throw errorFromResponse(response.status, payload);
    }
    return payload;
  }
}

async function parseJsonResponse(response: Response): Promise<JsonObject> {
  let payload: unknown;
  try {
    payload = await response.json();
  } catch (error) {
    throw new BeeCrawlError("BeeCrawl returned invalid JSON", {
      statusCode: response.status,
      cause: error,
    });
  }
  if (!isJsonObject(payload)) {
    throw new BeeCrawlError("BeeCrawl returned a non-object JSON response", {
      statusCode: response.status,
    });
  }
  return payload;
}

function errorFromResponse(statusCode: number, payload: JsonObject): BeeCrawlError {
  const detail = payload.detail ?? payload;
  const message = isJsonObject(detail) && typeof detail.message === "string"
    ? detail.message
    : String(detail || "BeeCrawl request failed");
  return new BeeCrawlError(message, { statusCode, detail });
}

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
