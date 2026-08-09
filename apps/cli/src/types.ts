import type { BeeCrawlClient, BeeCrawlClientOptions, JsonObject } from "beecrawl-sdk";

export type FlagValues = Map<string, string[]>;

export interface ParsedArgs {
  flags: FlagValues;
  positionals: string[];
  has(name: string): boolean;
  get(name: string): string | undefined;
  getAll(name: string): string[];
  boolean(name: string): boolean;
}

export interface CommandIO {
  stdout: { write(value: string): void };
  stderr: { write(value: string): void };
  stdin?: AsyncIterable<Uint8Array>;
}

export interface BeeCrawlClientLike {
  v2Search(query: string, options?: JsonObject): Promise<JsonObject>;
  v2Scrape(url: string, options?: JsonObject): Promise<JsonObject>;
  v2Map(url: string, options?: JsonObject): Promise<JsonObject>;
  v2Extract(urls: string[], options?: JsonObject): Promise<JsonObject>;
  v2Crawl(url: string, options?: JsonObject): Promise<JsonObject>;
  v2JobStatus(kind: "crawl" | "batch/scrape", jobId: string, params?: Record<string, string | number>): Promise<JsonObject>;
  cancelV2Job(kind: "crawl" | "batch/scrape", jobId: string): Promise<JsonObject>;
  createAgent(prompt: string, options?: JsonObject): Promise<JsonObject>;
  getAgent(jobId: string): Promise<JsonObject>;
  cancelAgent(jobId: string): Promise<JsonObject>;
}

export type ClientFactory = (options: BeeCrawlClientOptions) => BeeCrawlClientLike;

export interface CliDependencies {
  env?: NodeJS.ProcessEnv;
  homeDir?: string;
  cwd?: string;
  platform?: NodeJS.Platform;
  configPath?: string;
  clientFactory?: ClientFactory;
  fetch?: typeof fetch;
  sleep?: (milliseconds: number) => Promise<void>;
  openBrowser?: (url: string) => Promise<void>;
}
