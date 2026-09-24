import { randomUUID } from "node:crypto";
import { promises as fs } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

export const CONSENT_TIMEOUT_MS = 15_000;
const MAX_CONSENT_TIMEOUT_MS = 86_400_000;

/** Read the native router's consent timeout for each request (JSON or TOML). */
export async function consentTimeoutMs(): Promise<number> {
  try {
    const filename = configFilePath();
    const contents = await fs.readFile(filename, "utf8");
    let timeout: unknown;
    if (filename.endsWith(".toml")) {
      const section = contents.match(/(?:^|\n)\s*\[timeouts\]([\s\S]*?)(?=\n\s*\[|$)/)?.[1] ?? "";
      const value = section.match(/^\s*consentMs\s*=\s*([0-9][0-9_]*)\s*(?:#.*)?$/m)?.[1];
      timeout = value === undefined ? undefined : Number(value.replaceAll("_", ""));
    } else {
      timeout = (JSON.parse(contents) as { timeouts?: { consentMs?: unknown } }).timeouts?.consentMs;
    }
    if (typeof timeout === "number" && Number.isSafeInteger(timeout) && timeout > 0 && timeout <= MAX_CONSENT_TIMEOUT_MS) return timeout;
  } catch { /* Unconfigured or invalid config uses the native default. */ }
  return CONSENT_TIMEOUT_MS;
}

export type ConsentRequest = {
  id: string;
  createdAt: number;
  expiresAt: number;
  current?: { provider?: string; model?: string; variant?: string };
  recommendation: { provider: string; model: string; variant?: string; routingClass?: string; reason?: string };
};

export function stateDirectory(): string {
  if (!process.env.XDG_STATE_HOME && process.platform === "win32") {
    return path.join(process.env.APPDATA || path.join(homedir(), "AppData", "Roaming"), "system1router");
  }
  return path.join(process.env.XDG_STATE_HOME || path.join(homedir(), ".local", "state"), "system1router");
}

export function consentDirectory(): string {
  return path.join(stateDirectory(), "opencode-consent");
}

export function providerLogPath(): string {
  return process.env.SYSTEM1ROUTER_VERBOSE_LOG
    ? path.resolve(process.env.SYSTEM1ROUTER_VERBOSE_LOG)
    : path.join(stateDirectory(), "provider-debug.jsonl");
}

export function configFilePath(): string {
  if (process.env.SYSTEM1ROUTER_CONFIG) return path.resolve(process.env.SYSTEM1ROUTER_CONFIG);
  const base = process.platform === "win32"
    ? process.env.APPDATA || path.join(homedir(), "AppData", "Roaming")
    : process.env.XDG_CONFIG_HOME || path.join(homedir(), ".config");
  return path.join(base, "system1router", "config.json");
}

export async function routerEnabled(): Promise<boolean> {
  if (process.env.SYSTEM1ROUTER_OPENCODE_DISABLED === "1") return false;
  try {
    const filename = configFilePath();
    const contents = await fs.readFile(filename, "utf8");
    if (filename.endsWith(".toml")) {
      const section = contents.match(/(?:^|\n)\s*\[routing\]([\s\S]*?)(?=\n\s*\[|$)/)?.[1] ?? "";
      return /^\s*enabled\s*=\s*true\s*(?:#.*)?$/m.test(section);
    }
    return (JSON.parse(contents) as { routing?: { enabled?: boolean } }).routing?.enabled === true;
  } catch {
    return false;
  }
}

export async function verboseEnabled(): Promise<boolean> {
  if (process.env.SYSTEM1ROUTER_VERBOSE !== undefined) return process.env.SYSTEM1ROUTER_VERBOSE === "1";
  try {
    const value = JSON.parse(await fs.readFile(path.join(consentDirectory(), "verbose.json"), "utf8")) as { enabled?: boolean };
    return value.enabled === true;
  } catch {
    return false;
  }
}

export async function writeJsonAtomic(filename: string, value: unknown): Promise<void> {
  const directory = path.dirname(filename);
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  await fs.chmod(directory, 0o700).catch(() => {});
  const temporary = `${filename}.${randomUUID()}.tmp`;
  try {
    await fs.writeFile(temporary, JSON.stringify(value), { mode: 0o600 });
    await fs.rename(temporary, filename);
  } catch (error) {
    await fs.rm(temporary, { force: true }).catch(() => {});
    throw error;
  }
}

export type Usage = {
  host: string;
  model?: string;
  suggestedModel?: string;
  accepted?: boolean;
  policyOutcome?: string;
};

export async function recordUsage(entry: Usage): Promise<void> {
  if (process.env.SYSTEM1ROUTER_TELEMETRY_DISABLED !== undefined) return;
  try {
    const filename = configFilePath();
    const config = await fs.readFile(filename, "utf8");
    if (filename.endsWith(".toml")) {
      const section = config.match(/(?:^|\n)\s*\[telemetry\]([\s\S]*?)(?=\n\s*\[|$)/)?.[1] ?? "";
      if (/^\s*enabled\s*=\s*false\s*(?:#.*)?$/m.test(section)) return;
    } else if ((JSON.parse(config) as { telemetry?: { enabled?: boolean } }).telemetry?.enabled === false) return;
  } catch { /* The core handles config errors; keep the current turn fail-open. */ }
  const directory = stateDirectory();
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const filename = path.join(directory, "usage.jsonl");
  const handle = await fs.open(filename, "a", 0o600);
  try {
    await handle.writeFile(`${JSON.stringify({ ...entry, timestampMs: Date.now() })}\n`);
  } finally {
    await handle.close();
  }
}

export type Stats = {
  calls: number;
  models: Record<string, number>;
  suggestions: number;
  accepted: number;
  declined: number;
};

export function summarizeUsage(entries: Usage[]): Stats {
  const stats: Stats = { calls: 0, models: {}, suggestions: 0, accepted: 0, declined: 0 };
  for (const entry of entries) {
    stats.calls++;
    const model = entry.model || "unknown";
    stats.models[model] = (stats.models[model] ?? 0) + 1;
    if (entry.suggestedModel && typeof entry.accepted === "boolean") {
      stats.suggestions++;
      if (entry.accepted) stats.accepted++;
      else stats.declined++;
    }
  }
  return stats;
}

export async function readStats(): Promise<Stats> {
  const files = [
    { filename: path.join(stateDirectory(), "usage.jsonl"), native: false },
    { filename: process.env.SYSTEM1ROUTER_TELEMETRY || path.join(stateDirectory(), "events.jsonl"), native: true },
  ];
  const entries: Usage[] = [];
  for (const file of files) {
    try {
      const lines = (await fs.readFile(file.filename, "utf8")).split("\n");
      for (const line of lines) {
        if (!line) continue;
        try {
          const entry = JSON.parse(line) as Usage;
          // The native core records OpenCode's pending decision; the server
          // records the actual choice once the TUI has answered it.
          if (file.native && entry.host === "opencode") continue;
          entries.push(entry);
        } catch { /* A partially written line must not hide earlier stats. */ }
      }
    } catch { /* No history yet. */ }
  }
  return summarizeUsage(entries);
}
