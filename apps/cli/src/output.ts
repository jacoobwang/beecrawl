import type { CommandIO } from "./types.js";
import { usageError } from "./errors.js";

export type OutputFormat = "json" | "markdown";

export function outputFormat(value: string | undefined, json: boolean, command: string): OutputFormat {
  if (json) return "json";
  if (value === undefined) return command === "scrape" ? "markdown" : "json";
  if (value !== "json" && value !== "markdown") throw usageError("--format must be json or markdown");
  return value;
}

export function renderResult(value: unknown, format: OutputFormat): string {
  if (format === "json") return `${JSON.stringify(value, null, 2)}\n`;
  if (typeof value === "string") return ensureTrailingNewline(value);
  const markdown = findMarkdown(value);
  if (markdown !== undefined) return ensureTrailingNewline(markdown);
  return ensureTrailingNewline(JSON.stringify(value, null, 2));
}

export function writeResult(io: CommandIO, value: unknown, format: OutputFormat): void {
  io.stdout.write(renderResult(value, format));
}

function findMarkdown(value: unknown): string | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findMarkdown(item);
      if (found !== undefined) return found;
    }
    return undefined;
  }
  const record = value as Record<string, unknown>;
  if (typeof record.markdown === "string") return record.markdown;
  for (const child of Object.values(record)) {
    const found = findMarkdown(child);
    if (found !== undefined) return found;
  }
  return undefined;
}

function ensureTrailingNewline(value: string): string {
  return value.endsWith("\n") ? value : `${value}\n`;
}
