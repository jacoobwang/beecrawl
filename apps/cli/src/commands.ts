import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { BeeCrawlClient } from "beecrawl-sdk";
import { login, openBrowser } from "./auth.js";
import { ConfigStore, defaultConfigPath, resolveCredentials, type CredentialProfile } from "./config.js";
import { authError, CliError, ExitCode, notFoundError, usageError, writeError } from "./errors.js";
import { readOptions, numberFlag, parseJsonValue, setBooleanFlag, setNumberFlag, setRepeatFlag, type OptionsObject } from "./options.js";
import { outputFormat, writeResult } from "./output.js";
import { parseArgs } from "./parser.js";
import { initSkills } from "./skill.js";
import type { BeeCrawlClientLike, CliDependencies, CommandIO, ParsedArgs } from "./types.js";

const DEFAULT_DASHBOARD_URL = "https://dashboard.beecrawl.dev";
const DEFAULT_WAIT_TIMEOUT_MS = 300_000;
const DEFAULT_POLL_INTERVAL_MS = 1_000;

const HELP = `BeeCrawl CLI

Usage:
  beecrawl search <query> [options]
  beecrawl scrape <url> [options]
  beecrawl map <url> [options]
  beecrawl extract <url> [url...] [options]
  beecrawl crawl <url> [options]
  beecrawl crawl start|status|cancel ...
  beecrawl agent <prompt> [options]
  beecrawl agent start|status|cancel ...
  beecrawl login [--profile <name>]
  beecrawl logout [--profile <name>]
  beecrawl profile list|current|use|remove ...
  beecrawl init --all|--agent <claude-code|codex|opencode>

Global options:
  --profile <name>       Select a local credential profile
  --api-key <key>        Use an API key without saving it
  --base-url <url>       Override the BeeCrawl API URL
  --dashboard-url <url>  Override the Dashboard URL used by login
  --format json|markdown  Choose the final output representation
  --json                 Emit JSON output and JSON errors
  --options <json>       Pass advanced API options as JSON
  --options-file <path>  Read advanced API options from a JSON file or stdin (-)
  --wait-timeout-ms <n>  Override the default long-running Job wait timeout
  --poll-interval-ms <n> Override the default Job polling interval
  --help                 Show this help
`;

export function defaultIO(): CommandIO {
  return { stdout: process.stdout, stderr: process.stderr, stdin: process.stdin };
}

export async function runCli(argv: string[], io: CommandIO = defaultIO(), dependencies: CliDependencies = {}): Promise<number> {
  const parsed = parseArgs(argv);
  const jsonErrors = parsed.boolean("json");
  try {
    if (parsed.boolean("version")) {
      io.stdout.write("0.1.0\n");
      return ExitCode.Success;
    }
    if (parsed.boolean("help") || parsed.positionals.length === 0) {
      io.stdout.write(HELP);
      return ExitCode.Success;
    }
    return await execute(parsed, io, dependencies);
  } catch (error) {
    return writeError(io, error, jsonErrors);
  }
}

async function execute(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const first = parsed.positionals[0];
  if (isHttpUrl(first)) return runScrape(parsed, parsed.positionals, io, dependencies);
  switch (first) {
    case "search": return runSearch(parsed, io, dependencies);
    case "scrape": return runScrape(parsed, parsed.positionals.slice(1), io, dependencies);
    case "map": return runMap(parsed, io, dependencies);
    case "extract": return runExtract(parsed, io, dependencies);
    case "crawl": return runCrawl(parsed, io, dependencies);
    case "agent": return runAgent(parsed, io, dependencies);
    case "login": return runLogin(parsed, io, dependencies);
    case "logout": return runLogout(parsed, io, dependencies);
    case "profile": return runProfile(parsed, io, dependencies);
    case "init": return runInit(parsed, io, dependencies);
    default: throw usageError(`Unknown command: ${first}`);
  }
}

async function runSearch(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const query = requiredText(parsed.positionals.slice(1), "search query");
  const options = await readOptions(parsed, io);
  setNumberFlag(options, parsed, "limit", "limit", 1);
  setRepeatFlag(options, parsed, "source", "sources");
  setRepeatFlag(options, parsed, "category", "categories");
  setRepeatFlag(options, parsed, "include-domain", "includeDomains");
  setRepeatFlag(options, parsed, "exclude-domain", "excludeDomains");
  setStringFlag(options, parsed, "lang", "lang");
  setStringFlag(options, parsed, "country", "country");
  setStringFlag(options, parsed, "location", "location");
  const result = await clientFor(parsed, dependencies).v2Search(query, options);
  writeResult(io, result, outputFormat(parsed.get("format"), parsed.boolean("json"), "search"));
  return ExitCode.Success;
}

async function runScrape(parsed: ParsedArgs, urlsOrPositionals: string[], io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const url = requiredUrl(urlsOrPositionals[0]);
  const options = await readOptions(parsed, io);
  applyScrapeFlags(options, parsed);
  const result = await clientFor(parsed, dependencies).v2Scrape(url, options);
  writeResult(io, result, outputFormat(parsed.get("format"), parsed.boolean("json"), "scrape"));
  return ExitCode.Success;
}

async function runMap(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const url = requiredUrl(parsed.positionals[1]);
  const options = await readOptions(parsed, io);
  setNumberFlag(options, parsed, "limit", "limit", 1);
  setStringFlag(options, parsed, "search", "search");
  setBooleanFlag(options, parsed, "include-subdomains", "includeSubdomains");
  setStringFlag(options, parsed, "sitemap", "sitemap");
  setBooleanFlag(options, parsed, "ignore-query-parameters", "ignoreQueryParameters");
  const result = await clientFor(parsed, dependencies).v2Map(url, options);
  writeResult(io, result, outputFormat(parsed.get("format"), parsed.boolean("json"), "map"));
  return ExitCode.Success;
}

async function runExtract(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const urls = parsed.positionals.slice(1).filter((value) => !value.startsWith("-"));
  if (!urls.length || urls.some((url) => !isHttpUrl(url))) throw usageError("extract requires one or more valid URLs");
  const options = await readOptions(parsed, io);
  const schema = parsed.get("schema") ?? (parsed.get("schema-file") ? await readFile(parsed.get("schema-file")!, "utf8").catch((error: unknown) => { throw usageError(`Unable to read schema file: ${error instanceof Error ? error.message : String(error)}`); }) : undefined);
  if (schema !== undefined) options.schema = typeof schema === "string" && parsed.has("schema-file") ? parseJsonValue(schema, "schema file") : parseJsonValue(schema, "--schema");
  const prompt = await promptValue(parsed, io);
  if (prompt !== undefined) options.prompt = prompt;
  if (options.schema === undefined && options.prompt === undefined) throw usageError("extract requires --schema, --schema-file, or --prompt");
  setBooleanFlag(options, parsed, "enable-web-search", "enableWebSearch");
  setBooleanFlag(options, parsed, "show-sources", "showSources");
  const result = await clientFor(parsed, dependencies).v2Extract(urls, options);
  writeResult(io, result, outputFormat(parsed.get("format"), parsed.boolean("json"), "extract"));
  return ExitCode.Success;
}

async function runCrawl(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const lifecycle = parsed.positionals[1];
  if (lifecycle === "status" || lifecycle === "cancel") {
    const jobId = requiredText([parsed.positionals[2]], `crawl ${lifecycle} job ID`);
    const client = clientFor(parsed, dependencies);
    const result = lifecycle === "status" ? await readWithRetry(() => client.v2JobStatus("crawl", jobId), dependencies, io) : await client.cancelV2Job("crawl", jobId);
    writeResult(io, result, outputFormat(parsed.get("format"), parsed.boolean("json"), "crawl"));
    return lifecycle === "status" ? ExitCode.Success : ExitCode.Success;
  }
  const startOnly = lifecycle === "start" || parsed.boolean("no-wait");
  const url = requiredUrl(lifecycle === "start" ? parsed.positionals[2] : parsed.positionals[1]);
  const options = await readOptions(parsed, io);
  applyCrawlFlags(options, parsed);
  const client = clientFor(parsed, dependencies);
  const created = await client.v2Crawl(url, options);
  const jobId = requiredJobId(created);
  if (startOnly) {
    writeResult(io, created, outputFormat(parsed.get("format"), parsed.boolean("json"), "crawl"));
    return ExitCode.Success;
  }
  const result = await waitForJob("crawl", jobId, () => client.v2JobStatus("crawl", jobId), parsed, dependencies, io);
  writeResult(io, result, outputFormat(parsed.get("format"), parsed.boolean("json"), "crawl"));
  return ExitCode.Success;
}

async function runAgent(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const lifecycle = parsed.positionals[1];
  if (lifecycle === "status" || lifecycle === "cancel") {
    const jobId = requiredText([parsed.positionals[2]], `agent ${lifecycle} job ID`);
    const client = clientFor(parsed, dependencies);
    const result = lifecycle === "status" ? await readWithRetry(() => client.getAgent(jobId), dependencies, io) : await client.cancelAgent(jobId);
    writeResult(io, result, outputFormat(parsed.get("format"), parsed.boolean("json"), "agent"));
    return ExitCode.Success;
  }
  const startOnly = lifecycle === "start" || parsed.boolean("no-wait");
  const promptPositionals = lifecycle === "start" ? parsed.positionals.slice(2) : parsed.positionals.slice(1);
  const prompt = await promptValue(parsed, io) ?? requiredText(promptPositionals, "agent prompt");
  const options = await readOptions(parsed, io);
  const urls = parsed.getAll("url");
  if (urls.length) options.urls = urls;
  setNumberFlag(options, parsed, "max-credits", "maxCredits", 1);
  const client = clientFor(parsed, dependencies);
  const created = await client.createAgent(prompt, options);
  const jobId = requiredJobId(created);
  if (startOnly) {
    writeResult(io, created, outputFormat(parsed.get("format"), parsed.boolean("json"), "agent"));
    return ExitCode.Success;
  }
  const result = await waitForJob("agent", jobId, () => client.getAgent(jobId), parsed, dependencies, io);
  writeResult(io, result, outputFormat(parsed.get("format"), parsed.boolean("json"), "agent"));
  return ExitCode.Success;
}

async function runLogin(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const store = configStore(dependencies);
  const profileName = parsed.get("profile") ?? "default";
  const dashboardUrl = parsed.get("dashboard-url") ?? dependencies.env?.BEECRAWL_DASHBOARD_URL ?? process.env.BEECRAWL_DASHBOARD_URL ?? DEFAULT_DASHBOARD_URL;
  const result = await login({ dashboardUrl, profileName, noBrowser: parsed.boolean("no-browser"), store, io, openBrowser: dependencies.openBrowser ?? openBrowser, fetch: dependencies.fetch });
  writeResult(io, { profile: result.profileName, apiUrl: result.profile.apiUrl, workspaceId: result.profile.workspaceId }, "json");
  return ExitCode.Success;
}

async function runLogout(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  if (parsed.has("revoke")) throw usageError("BeeCrawl CLI logout is local-only; remote key revocation is managed by the Dashboard");
  const store = configStore(dependencies);
  const config = await store.load();
  const name = parsed.get("profile") ?? config.currentProfile;
  if (!name) throw notFoundError("No active BeeCrawl profile");
  await store.removeProfile(name);
  writeResult(io, { profile: name, loggedOut: true, remoteKeyRevoked: false }, "json");
  return ExitCode.Success;
}

async function runProfile(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const store = configStore(dependencies);
  const config = await store.load();
  const action = parsed.positionals[1];
  if (action === "list") {
    writeResult(io, Object.entries(config.profiles).map(([name, profile]) => ({ name, apiUrl: profile.apiUrl, workspaceId: profile.workspaceId, source: profile.source, current: name === config.currentProfile })), "json");
    return ExitCode.Success;
  }
  if (action === "current") {
    const name = config.currentProfile;
    writeResult(io, name && config.profiles[name] ? { name, apiUrl: config.profiles[name].apiUrl, workspaceId: config.profiles[name].workspaceId, source: config.profiles[name].source } : null, "json");
    return ExitCode.Success;
  }
  if (action === "use") {
    const name = requiredText([parsed.positionals[2]], "profile name");
    await store.useProfile(name);
    writeResult(io, { profile: name, current: true }, "json");
    return ExitCode.Success;
  }
  if (action === "remove") {
    const name = requiredText([parsed.positionals[2]], "profile name");
    await store.removeProfile(name);
    writeResult(io, { profile: name, removed: true, remoteKeyRevoked: false }, "json");
    return ExitCode.Success;
  }
  throw usageError("profile requires list, current, use, or remove");
}

async function runInit(parsed: ParsedArgs, io: CommandIO, dependencies: CliDependencies): Promise<number> {
  const agents = parsed.get("agent") ? [parsed.get("agent")!] : [];
  if (!parsed.has("all") && !agents.length) throw usageError("init requires --all or --agent");
  const scope = parsed.get("scope") ?? "global";
  const result = await initSkills({ agents, scope: scope as "global" | "project", force: parsed.boolean("force"), dryRun: parsed.boolean("dry-run"), homeDir: dependencies.homeDir ?? homedir(), cwd: dependencies.cwd ?? process.cwd() });
  writeResult(io, result, parsed.boolean("json") ? "json" : "json");
  return ExitCode.Success;
}

function clientFor(parsed: ParsedArgs, dependencies: CliDependencies): BeeCrawlClientLike {
  const env = dependencies.env ?? process.env;
  const store = configStore(dependencies);
  return new LazyClient(() => resolveCredentials(store, { profile: parsed.get("profile"), apiKey: parsed.get("api-key"), baseUrl: parsed.get("base-url") }, env).then((credentials) => (dependencies.clientFactory ?? ((options) => new BeeCrawlClient(options)))({ apiKey: credentials.apiKey, baseUrl: credentials.apiUrl })));
}

class LazyClient implements BeeCrawlClientLike {
  private promise?: Promise<BeeCrawlClientLike>;
  constructor(private readonly factory: () => Promise<BeeCrawlClientLike>) {}
  private get(): Promise<BeeCrawlClientLike> { return this.promise ??= this.factory(); }
  v2Search(...args: Parameters<BeeCrawlClientLike["v2Search"]>) { return this.get().then((client) => client.v2Search(...args)); }
  v2Scrape(...args: Parameters<BeeCrawlClientLike["v2Scrape"]>) { return this.get().then((client) => client.v2Scrape(...args)); }
  v2Map(...args: Parameters<BeeCrawlClientLike["v2Map"]>) { return this.get().then((client) => client.v2Map(...args)); }
  v2Extract(...args: Parameters<BeeCrawlClientLike["v2Extract"]>) { return this.get().then((client) => client.v2Extract(...args)); }
  v2Crawl(...args: Parameters<BeeCrawlClientLike["v2Crawl"]>) { return this.get().then((client) => client.v2Crawl(...args)); }
  v2JobStatus(...args: Parameters<BeeCrawlClientLike["v2JobStatus"]>) { return this.get().then((client) => client.v2JobStatus(...args)); }
  cancelV2Job(...args: Parameters<BeeCrawlClientLike["cancelV2Job"]>) { return this.get().then((client) => client.cancelV2Job(...args)); }
  createAgent(...args: Parameters<BeeCrawlClientLike["createAgent"]>) { return this.get().then((client) => client.createAgent(...args)); }
  getAgent(...args: Parameters<BeeCrawlClientLike["getAgent"]>) { return this.get().then((client) => client.getAgent(...args)); }
  cancelAgent(...args: Parameters<BeeCrawlClientLike["cancelAgent"]>) { return this.get().then((client) => client.cancelAgent(...args)); }
}

function configStore(dependencies: CliDependencies): ConfigStore {
  return new ConfigStore(dependencies.configPath ?? defaultConfigPath(dependencies.platform ?? process.platform, dependencies.homeDir ?? homedir(), dependencies.env ?? process.env));
}

function applyScrapeFlags(options: OptionsObject, parsed: ParsedArgs): void {
  setRepeatFlag(options, parsed, "content-format", "formats");
  setNumberFlag(options, parsed, "timeout-ms", "timeout", 1);
  setNumberFlag(options, parsed, "wait-for-ms", "waitFor", 0);
  setBooleanFlag(options, parsed, "only-main-content", "onlyMainContent");
  setBooleanFlag(options, parsed, "only-clean-content", "onlyCleanContent");
  setRepeatFlag(options, parsed, "include-tag", "includeTags");
  setRepeatFlag(options, parsed, "exclude-tag", "excludeTags");
}

function applyCrawlFlags(options: OptionsObject, parsed: ParsedArgs): void {
  setNumberFlag(options, parsed, "limit", "limit", 1);
  setNumberFlag(options, parsed, "max-depth", "maxDiscoveryDepth", 0);
  setRepeatFlag(options, parsed, "include-path", "includePaths");
  setRepeatFlag(options, parsed, "exclude-path", "excludePaths");
  setBooleanFlag(options, parsed, "include-subdomains", "allowSubdomains");
  setNumberFlag(options, parsed, "max-concurrency", "maxConcurrency", 1);
  setStringFlag(options, parsed, "sitemap", "sitemap");
  setBooleanFlag(options, parsed, "ignore-query-parameters", "ignoreQueryParameters");
  const hasScrape = ["content-format", "timeout-ms", "wait-for-ms", "only-main-content", "only-clean-content", "include-tag", "exclude-tag"].some((flag) => parsed.has(flag));
  if (hasScrape || typeof options.scrapeOptions === "object") {
    const scrapeOptions = typeof options.scrapeOptions === "object" && options.scrapeOptions !== null && !Array.isArray(options.scrapeOptions) ? options.scrapeOptions as OptionsObject : {};
    applyScrapeFlags(scrapeOptions, parsed);
    options.scrapeOptions = scrapeOptions;
  }
}

function setStringFlag(options: OptionsObject, parsed: ParsedArgs, flag: string, key = flag): void {
  const value = parsed.get(flag);
  if (value !== undefined) options[key] = value;
}

async function promptValue(parsed: ParsedArgs, io: CommandIO): Promise<string | undefined> {
  const inline = parsed.get("prompt");
  if (inline !== undefined) return inline;
  const file = parsed.get("prompt-file");
  if (!file) return undefined;
  if (file === "-") {
    if (!io.stdin) throw usageError("stdin is unavailable for --prompt-file -");
    const chunks: Uint8Array[] = [];
    for await (const chunk of io.stdin) chunks.push(chunk);
    return Buffer.concat(chunks).toString("utf8");
  }
  return readFile(file, "utf8").catch((error: unknown) => { throw usageError(`Unable to read prompt file ${file}: ${error instanceof Error ? error.message : String(error)}`); });
}

async function waitForJob(
  kind: "crawl" | "agent",
  jobId: string,
  read: () => Promise<Record<string, unknown>>,
  parsed: ParsedArgs,
  dependencies: CliDependencies,
  io: CommandIO,
): Promise<Record<string, unknown>> {
  const timeoutMs = numberFlag(parsed, "wait-timeout-ms", 1) ?? DEFAULT_WAIT_TIMEOUT_MS;
  const intervalMs = numberFlag(parsed, "poll-interval-ms", 0) ?? DEFAULT_POLL_INTERVAL_MS;
  const sleep = dependencies.sleep ?? ((milliseconds: number) => new Promise<void>((resolve) => setTimeout(resolve, milliseconds)));
  const deadline = Date.now() + timeoutMs;
  while (true) {
    const result = await readWithRetry(read, dependencies, io);
    const status = String(result.status ?? "").toLowerCase();
    if (["completed", "failed", "cancelled", "error"].includes(status)) {
      if (status !== "completed") throw new CliError(`${kind} Job ${jobId} ended with status ${status}`, ExitCode.Job, { jobId, result });
      return result;
    }
    if (Date.now() >= deadline) throw new CliError(`Timed out waiting for ${kind} Job ${jobId}`, ExitCode.Job, { jobId, status: result.status });
    if (intervalMs > 0) await sleep(intervalMs);
  }
}

async function readWithRetry<T>(read: () => Promise<T>, dependencies: CliDependencies, _io: CommandIO): Promise<T> {
  const sleep = dependencies.sleep ?? ((milliseconds: number) => new Promise<void>((resolve) => setTimeout(resolve, milliseconds)));
  for (let attempt = 0; ; attempt += 1) {
    try {
      return await read();
    } catch (error) {
      const status = error instanceof Error && "statusCode" in error ? Number((error as { statusCode?: number }).statusCode) : undefined;
      if (attempt >= 2 || (status !== undefined && status < 500 && status !== 429)) throw error;
      await sleep(Math.min(250 * (2 ** attempt), 1_000));
    }
  }
}

function requiredText(values: Array<string | undefined>, label: string): string {
  const value = values.filter((item): item is string => Boolean(item)).join(" ").trim();
  if (!value) throw usageError(`${label} is required`);
  return value;
}

function requiredUrl(value: string | undefined): string {
  if (!value || !isHttpUrl(value)) throw usageError("A valid http(s) URL is required");
  return value;
}

function requiredJobId(value: Record<string, unknown>): string {
  if (typeof value.id !== "string" || !value.id) throw new CliError("BeeCrawl did not return a Job ID", ExitCode.Network, value);
  return value.id;
}

function isHttpUrl(value: string | undefined): value is string {
  if (!value) return false;
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}
