import { chmod, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { randomUUID } from "node:crypto";
import { authError, notFoundError } from "./errors.js";

export interface CredentialProfile {
  apiUrl: string;
  apiKey: string;
  workspaceId?: string;
  source?: string;
  createdAt?: string;
}

export interface ConfigFile {
  version: 1;
  currentProfile?: string;
  profiles: Record<string, CredentialProfile>;
}

export function defaultConfigPath(
  platform: NodeJS.Platform = process.platform,
  home = homedir(),
  env: NodeJS.ProcessEnv = process.env,
): string {
  if (platform === "darwin") return join(home, "Library", "Application Support", "beecrawl", "config.json");
  if (platform === "win32") return join(env.APPDATA ?? join(home, "AppData", "Roaming"), "beecrawl", "config.json");
  return join(env.XDG_CONFIG_HOME ?? join(home, ".config"), "beecrawl", "config.json");
}

export class ConfigStore {
  constructor(public readonly path: string) {}

  async load(): Promise<ConfigFile> {
    try {
      const raw = await readFile(this.path, "utf8");
      const parsed = JSON.parse(raw) as Partial<ConfigFile>;
      if (parsed.version !== 1 || typeof parsed.profiles !== "object" || parsed.profiles === null) {
        throw new Error("configuration must have version 1 and a profiles object");
      }
      return { version: 1, currentProfile: parsed.currentProfile, profiles: parsed.profiles as Record<string, CredentialProfile> };
    } catch (error: unknown) {
      if (isMissingFile(error)) return { version: 1, profiles: {} };
      throw authError(`Unable to read BeeCrawl configuration: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  async save(config: ConfigFile): Promise<void> {
    const directory = dirname(this.path);
    await mkdir(directory, { recursive: true, mode: 0o700 });
    const temporary = join(directory, `.config.${randomUUID()}.tmp`);
    await writeFile(temporary, `${JSON.stringify(config, null, 2)}\n`, { encoding: "utf8", mode: 0o600 });
    if (process.platform !== "win32") await chmod(temporary, 0o600);
    await rename(temporary, this.path);
  }

  async setProfile(name: string, profile: CredentialProfile): Promise<void> {
    const config = await this.load();
    config.profiles[name] = profile;
    config.currentProfile = name;
    await this.save(config);
  }

  async useProfile(name: string): Promise<void> {
    const config = await this.load();
    if (!config.profiles[name]) throw notFoundError(`Profile not found: ${name}`);
    config.currentProfile = name;
    await this.save(config);
  }

  async removeProfile(name: string): Promise<void> {
    const config = await this.load();
    if (!config.profiles[name]) throw notFoundError(`Profile not found: ${name}`);
    delete config.profiles[name];
    if (config.currentProfile === name) config.currentProfile = Object.keys(config.profiles)[0];
    await this.save(config);
  }
}

export interface CredentialResolution {
  apiUrl: string;
  apiKey: string;
  profileName?: string;
  workspaceId?: string;
}

export async function resolveCredentials(
  store: ConfigStore,
  flags: { profile?: string; apiKey?: string; baseUrl?: string },
  env: NodeJS.ProcessEnv = process.env,
): Promise<CredentialResolution> {
  const config = await store.load();
  const profileName = flags.profile ?? env.BEECRAWL_PROFILE ?? config.currentProfile ?? "default";
  const profile = config.profiles[profileName];
  const apiKey = flags.apiKey ?? env.BEECRAWL_API_KEY ?? profile?.apiKey;
  if (!apiKey) throw authError("No BeeCrawl API key is configured. Run `beecrawl login` or set BEECRAWL_API_KEY.");
  return {
    apiKey,
    apiUrl: flags.baseUrl ?? env.BEECRAWL_BASE_URL ?? profile?.apiUrl ?? "https://api.beecrawl.dev",
    profileName: profile ? profileName : undefined,
    workspaceId: profile?.workspaceId,
  };
}

function isMissingFile(error: unknown): error is NodeJS.ErrnoException {
  return typeof error === "object" && error !== null && "code" in error && error.code === "ENOENT";
}
