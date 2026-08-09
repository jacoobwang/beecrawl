import { createHash, randomBytes } from "node:crypto";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { spawn } from "node:child_process";
import { authError } from "./errors.js";
import type { CredentialProfile, ConfigStore } from "./config.js";
import type { CommandIO } from "./types.js";

export interface LoginOptions {
  dashboardUrl: string;
  profileName: string;
  noBrowser: boolean;
  store: ConfigStore;
  io: CommandIO;
  openBrowser?: (url: string) => Promise<void>;
  fetch?: typeof fetch;
}

export async function login(options: LoginOptions): Promise<{ profileName: string; profile: CredentialProfile }> {
  const state = randomBytes(24).toString("base64url");
  const codeVerifier = randomBytes(32).toString("base64url");
  const codeChallenge = createHash("sha256").update(codeVerifier).digest("base64url");
  const server = createServer((request, response) => handleCallback(request, response, state, callbackPromiseResolve, callbackPromiseReject));
  let callbackPromiseResolve!: (value: CallbackResult) => void;
  let callbackPromiseReject!: (reason: unknown) => void;
  const callbackPromise = new Promise<CallbackResult>((resolve, reject) => {
    callbackPromiseResolve = resolve;
    callbackPromiseReject = reject;
  });

  await listen(server);
  const address = server.address();
  if (!address || typeof address === "string") {
    server.close();
    throw authError("Unable to determine the CLI callback address");
  }
  const redirectUri = `http://127.0.0.1:${address.port}/callback`;
  const dashboardUrl = options.dashboardUrl.replace(/\/+$/, "");
  const authorizeUrl = new URL(`${dashboardUrl}/cli/authorize`);
  authorizeUrl.searchParams.set("redirect_uri", redirectUri);
  authorizeUrl.searchParams.set("code_challenge", codeChallenge);
  authorizeUrl.searchParams.set("code_challenge_method", "S256");
  authorizeUrl.searchParams.set("state", state);
  authorizeUrl.searchParams.set("profile", options.profileName);

  try {
    if (options.noBrowser) {
      options.io.stderr.write(`Open this URL to authorize BeeCrawl CLI:\n${authorizeUrl}\n`);
    } else {
      await (options.openBrowser ?? openBrowser)(authorizeUrl.toString());
    }
    const callback = await callbackPromise;
    const response = await exchangeToken(dashboardUrl, callback.code, codeVerifier, redirectUri, state, options.fetch);
    const apiKey = stringField(response, "apiKey", "api_key");
    const apiUrl = stringField(response, "apiUrl", "api_url");
    if (!apiKey || !apiUrl) throw authError("Dashboard token exchange did not return an API key and API URL", response);
    const profile: CredentialProfile = {
      apiKey,
      apiUrl,
      workspaceId: stringField(response, "workspaceId", "workspace_id"),
      source: "dashboard-login",
      createdAt: new Date().toISOString(),
    };
    await options.store.setProfile(options.profileName, profile);
    return { profileName: options.profileName, profile };
  } finally {
    await closeServer(server);
  }
}

interface CallbackResult {
  code: string;
}

function handleCallback(
  request: IncomingMessage,
  response: ServerResponse,
  expectedState: string,
  resolve: (value: CallbackResult) => void,
  reject: (reason: unknown) => void,
): void {
  try {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (url.pathname !== "/callback") {
      response.writeHead(404).end("Not found");
      return;
    }
    const error = url.searchParams.get("error");
    if (error) {
      response.writeHead(400, { "Content-Type": "text/html; charset=utf-8" }).end("BeeCrawl authorization was denied. You can close this window.");
      reject(authError(`Dashboard authorization failed: ${error}`));
      return;
    }
    const state = url.searchParams.get("state");
    const code = url.searchParams.get("code");
    if (!code || state !== expectedState) {
      response.writeHead(400, { "Content-Type": "text/html; charset=utf-8" }).end("Invalid BeeCrawl authorization callback. You can close this window.");
      reject(authError("Invalid Dashboard authorization callback"));
      return;
    }
    response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" }).end("BeeCrawl CLI authorization complete. You can close this window.");
    resolve({ code });
  } catch (error) {
    reject(authError(`Invalid Dashboard authorization callback: ${error instanceof Error ? error.message : String(error)}`));
  }
}

async function exchangeToken(dashboardUrl: string, code: string, verifier: string, redirectUri: string, state: string, fetchImpl: typeof fetch = globalThis.fetch): Promise<Record<string, unknown>> {
  if (!fetchImpl) throw authError("A fetch implementation is required for Dashboard login");
  let response: Response;
  try {
    response = await fetchImpl(`${dashboardUrl}/api/v1/cli/token`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code, code_verifier: verifier, redirect_uri: redirectUri, state }),
    });
  } catch (error) {
    throw authError(`Dashboard token exchange failed: ${error instanceof Error ? error.message : String(error)}`);
  }
  const payload = await response.json().catch(() => ({})) as unknown;
  if (!response.ok || typeof payload !== "object" || payload === null) {
    throw authError(`Dashboard token exchange failed with HTTP ${response.status}`, payload);
  }
  return payload as Record<string, unknown>;
}

function stringField(record: Record<string, unknown>, ...names: string[]): string | undefined {
  for (const name of names) if (typeof record[name] === "string" && record[name]) return record[name] as string;
  return undefined;
}

async function listen(server: ReturnType<typeof createServer>): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.removeListener("error", reject);
      resolve();
    });
  });
}

async function closeServer(server: ReturnType<typeof createServer>): Promise<void> {
  if (!server.listening) return;
  await new Promise<void>((resolve) => server.close(() => resolve()));
}

export async function openBrowser(url: string): Promise<void> {
  const command = process.platform === "darwin" ? "open" : process.platform === "win32" ? "cmd" : "xdg-open";
  const args = process.platform === "win32" ? ["/c", "start", "", url] : [url];
  await new Promise<void>((resolve, reject) => {
    const child = spawn(command, args, { detached: true, stdio: "ignore" });
    child.once("error", reject);
    child.once("spawn", () => {
      child.unref();
      resolve();
    });
  }).catch((error) => {
    throw authError(`Unable to open a browser: ${error instanceof Error ? error.message : String(error)}. Re-run with --no-browser.`);
  });
}
