import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { usageError } from "./errors.js";

const MANAGED_MARKER = "<!-- Managed by beecrawl-cli; do not edit. -->";
const AGENTS = ["claude-code", "codex", "opencode"] as const;
export type SupportedAgent = (typeof AGENTS)[number];

export interface InitOptions {
  agents: string[];
  scope: "global" | "project";
  force: boolean;
  dryRun: boolean;
  homeDir: string;
  cwd: string;
}

export interface InitResult {
  agent: SupportedAgent;
  path: string;
  action: "installed" | "updated" | "skipped" | "would-install" | "would-update" | "would-skip";
}

export async function initSkills(options: InitOptions): Promise<InitResult[]> {
  const agents = options.agents.length ? options.agents : [...AGENTS];
  for (const agent of agents) if (!AGENTS.includes(agent as SupportedAgent)) throw usageError(`Unsupported agent: ${agent}`);
  if (options.scope !== "global" && options.scope !== "project") throw usageError("--scope must be global or project");
  const source = await readFile(join(dirname(fileURLToPath(import.meta.url)), "../skills/beecrawl/SKILL.md"), "utf8");
  const content = `${MANAGED_MARKER}\n\n${source.trim()}\n`;
  const results: InitResult[] = [];
  for (const agent of agents as SupportedAgent[]) {
    const target = skillPath(agent, options);
    const exists = await fileExists(target);
    const managed = exists && (await readFile(target, "utf8")).includes(MANAGED_MARKER);
    const action = !exists ? (options.dryRun ? "would-install" : "installed") : managed ? (options.dryRun ? "would-update" : "updated") : options.force ? (options.dryRun ? "would-update" : "updated") : (options.dryRun ? "would-skip" : "skipped");
    if ((!exists || managed || options.force) && !options.dryRun) {
      await mkdir(dirname(target), { recursive: true });
      await writeFile(target, content, { encoding: "utf8", mode: 0o644 });
    }
    results.push({ agent, path: target, action });
  }
  return results;
}

function skillPath(agent: SupportedAgent, options: InitOptions): string {
  const root = options.scope === "global" ? options.homeDir : options.cwd;
  const directory = options.scope === "global"
    ? { "claude-code": ".claude/skills", codex: ".codex/skills", opencode: ".config/opencode/skills" }[agent]
    : { "claude-code": ".claude/skills", codex: ".codex/skills", opencode: ".opencode/skills" }[agent];
  return join(root, directory, "beecrawl", "SKILL.md");
}

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}
