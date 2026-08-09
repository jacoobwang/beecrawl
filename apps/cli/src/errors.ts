import { BeeCrawlError } from "beecrawl-sdk";
import type { CommandIO } from "./types.js";

export const ExitCode = {
  Success: 0,
  Unknown: 1,
  Usage: 2,
  Auth: 3,
  NotFound: 4,
  RateLimit: 5,
  Network: 6,
  Job: 7,
} as const;

export type ExitCodeValue = (typeof ExitCode)[keyof typeof ExitCode];

export class CliError extends Error {
  constructor(
    message: string,
    public readonly exitCode: ExitCodeValue,
    public readonly detail?: unknown,
    options: { cause?: unknown; statusCode?: number } = {},
  ) {
    super(message, { cause: options.cause });
    this.name = "CliError";
    this.statusCode = options.statusCode;
  }

  public readonly statusCode?: number;
}

export function usageError(message: string): CliError {
  return new CliError(message, ExitCode.Usage);
}

export function authError(message: string, detail?: unknown): CliError {
  return new CliError(message, ExitCode.Auth, detail);
}

export function notFoundError(message: string): CliError {
  return new CliError(message, ExitCode.NotFound);
}

export function classifyError(error: unknown): CliError {
  if (error instanceof CliError) return error;
  if (error instanceof BeeCrawlError) {
    const status = error.statusCode;
    const exitCode = status === 401 || status === 403
      ? ExitCode.Auth
      : status === 404
        ? ExitCode.NotFound
        : status === 429
          ? ExitCode.RateLimit
          : status !== undefined && status >= 500
            ? ExitCode.Network
            : ExitCode.Unknown;
    return new CliError(error.message, exitCode, error.detail, { cause: error, statusCode: status });
  }
  if (error instanceof Error) {
    return new CliError(error.message, ExitCode.Unknown, undefined, { cause: error });
  }
  return new CliError(String(error), ExitCode.Unknown);
}

function errorCode(exitCode: ExitCodeValue): string {
  return {
    0: "success",
    1: "unknown_error",
    2: "usage_error",
    3: "auth_error",
    4: "not_found",
    5: "rate_limit_or_quota",
    6: "network_or_server_error",
    7: "job_failed_or_timeout",
  }[exitCode];
}

export function writeError(io: CommandIO, error: unknown, json: boolean): number {
  const cliError = classifyError(error);
  const payload = {
    error: errorCode(cliError.exitCode),
    message: cliError.message,
    code: cliError.exitCode,
    ...(cliError.statusCode === undefined ? {} : { status: cliError.statusCode }),
    ...(cliError.detail === undefined ? {} : { detail: cliError.detail }),
  };
  io.stderr.write(json ? `${JSON.stringify(payload)}\n` : `beecrawl: ${cliError.message}\n`);
  return cliError.exitCode;
}
