import type { ParsedArgs } from "./types.js";

const BOOLEAN_FLAGS = new Set([
  "help",
  "version",
  "json",
  "no-wait",
  "no-browser",
  "only-main-content",
  "only-clean-content",
  "include-subdomains",
  "ignore-query-parameters",
  "enable-web-search",
  "show-sources",
  "dry-run",
  "force",
]);

class ParsedArgsImpl implements ParsedArgs {
  constructor(public readonly flags: Map<string, string[]>, public readonly positionals: string[]) {}

  has(name: string): boolean {
    return this.flags.has(name);
  }

  get(name: string): string | undefined {
    const values = this.flags.get(name);
    return values?.at(-1);
  }

  getAll(name: string): string[] {
    return this.flags.get(name) ?? [];
  }

  boolean(name: string): boolean {
    const value = this.get(name);
    return this.has(name) && value !== "false";
  }
}

export function parseArgs(argv: string[]): ParsedArgs {
  const flags = new Map<string, string[]>();
  const positionals: string[] = [];
  let parseFlags = true;

  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (parseFlags && token === "--") {
      parseFlags = false;
      continue;
    }
    if (!parseFlags || !token.startsWith("--") || token === "-") {
      positionals.push(token);
      continue;
    }

    const withoutPrefix = token.slice(2);
    const equalsIndex = withoutPrefix.indexOf("=");
    const name = equalsIndex === -1 ? withoutPrefix : withoutPrefix.slice(0, equalsIndex);
    const inlineValue = equalsIndex === -1 ? undefined : withoutPrefix.slice(equalsIndex + 1);
    if (!name) {
      positionals.push(token);
      continue;
    }

    let value = inlineValue;
    if (value === undefined && !BOOLEAN_FLAGS.has(name) && argv[index + 1] !== undefined && !argv[index + 1].startsWith("--")) {
      value = argv[index + 1];
      index += 1;
    }
    const values = flags.get(name) ?? [];
    values.push(value ?? "true");
    flags.set(name, values);
  }

  return new ParsedArgsImpl(flags, positionals);
}

export const booleanFlags = BOOLEAN_FLAGS;
