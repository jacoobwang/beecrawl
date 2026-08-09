import { readFile } from "node:fs/promises";
import type { CommandIO, ParsedArgs } from "./types.js";
import { usageError } from "./errors.js";

export type OptionsObject = Record<string, unknown>;

function isObject(value: unknown): value is OptionsObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseJson(value: string, source: string): OptionsObject {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch (error) {
    throw usageError(`Invalid JSON in ${source}: ${error instanceof Error ? error.message : String(error)}`);
  }
  if (!isObject(parsed)) throw usageError(`${source} must contain a JSON object`);
  return parsed;
}

export async function readOptions(parsed: ParsedArgs, io: CommandIO): Promise<OptionsObject> {
  let options: OptionsObject = {};
  const optionsFile = parsed.get("options-file");
  if (optionsFile) {
    const content = optionsFile === "-"
      ? await readStdin(io)
      : await readFile(optionsFile, "utf8").catch((error: unknown) => {
          throw usageError(`Unable to read options file ${optionsFile}: ${error instanceof Error ? error.message : String(error)}`);
        });
    options = { ...options, ...parseJson(content, `options file ${optionsFile}`) };
  }
  const inline = parsed.get("options");
  if (inline) options = { ...options, ...parseJson(inline, "--options") };
  return options;
}

async function readStdin(io: CommandIO): Promise<string> {
  if (!io.stdin) throw usageError("stdin is unavailable for --options-file -");
  const chunks: Uint8Array[] = [];
  for await (const chunk of io.stdin) chunks.push(chunk);
  return Buffer.concat(chunks).toString("utf8");
}

export function numberFlag(parsed: ParsedArgs, name: string, minimum = 0): number | undefined {
  const value = parsed.get(name);
  if (value === undefined) return undefined;
  const number = Number(value);
  if (!Number.isFinite(number) || number < minimum) throw usageError(`--${name} must be a number >= ${minimum}`);
  return number;
}

export function setNumberFlag(options: OptionsObject, parsed: ParsedArgs, flag: string, key = flag, minimum = 0): void {
  const value = numberFlag(parsed, flag, minimum);
  if (value !== undefined) options[key] = value;
}

export function setBooleanFlag(options: OptionsObject, parsed: ParsedArgs, flag: string, key = flag): void {
  if (parsed.has(flag)) options[key] = parsed.boolean(flag);
}

export function setRepeatFlag(options: OptionsObject, parsed: ParsedArgs, flag: string, key = flag): void {
  if (parsed.has(flag)) options[key] = parsed.getAll(flag);
}

export function parseJsonValue(value: string, source: string): unknown {
  try {
    return JSON.parse(value);
  } catch (error) {
    throw usageError(`Invalid JSON in ${source}: ${error instanceof Error ? error.message : String(error)}`);
  }
}
