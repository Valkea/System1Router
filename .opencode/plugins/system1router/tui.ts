import { randomUUID } from "node:crypto";
import { promises as fs } from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import type { TuiPlugin } from "@opencode-ai/plugin/tui";
import { configFilePath, consentDirectory, providerLogPath, readStats, routerEnabled, verboseEnabled, writeJsonAtomic, type ConsentRequest } from "./shared";

function verboseStatePath(): string {
  return path.join(consentDirectory(), "verbose.json");
}

function formatModel(model?: ConsentRequest["current"]): string {
  if (!model?.model) return "the current selection";
  return `${model.provider ? `${model.provider}/` : ""}${model.model}${model.variant ? ` (${model.variant})` : ""}`;
}

async function writeConfigAtomic(filename: string, value: unknown): Promise<void> {
  await writeConfigTextAtomic(filename, `${JSON.stringify(value, null, 2)}\n`);
}

async function writeConfigTextAtomic(filename: string, contents: string): Promise<void> {
  const mode = (await fs.stat(filename)).mode & 0o777;
  const temporary = `${filename}.${randomUUID()}.tmp`;
  try {
    await fs.writeFile(temporary, contents, { mode });
    await fs.chmod(temporary, mode).catch(() => {});
    await fs.rename(temporary, filename);
  } catch (error) {
    await fs.rm(temporary, { force: true }).catch(() => {});
    throw error;
  }
}

function alert(api: Parameters<TuiPlugin>[0], title: string, message: string): void {
  api.ui.dialog.replace(
    () => api.ui.DialogAlert({ title, message, onConfirm: () => api.ui.dialog.clear() }),
    () => {},
  );
}

function writeOsc52(text: string): boolean {
  if (!process.stdout.isTTY) return false;
  const sequence = `\u001b]52;c;${Buffer.from(text).toString("base64")}\u0007`;
  const passthrough = `\u001bPtmux;\u001b${sequence}\u001b\\`;
  process.stdout.write(process.env.TMUX ? sequence + passthrough : process.env.STY ? passthrough : sequence);
  return true;
}

function copyWithCommand(command: string, args: string[], text: string): Promise<boolean> {
  return new Promise((resolve) => {
    const child = spawn(command, args, { stdio: ["pipe", "ignore", "ignore"] });
    child.once("error", () => resolve(false));
    child.once("close", (code) => resolve(code === 0));
    child.stdin?.end(text);
  });
}

async function copyToClipboard(text: string): Promise<boolean> {
  const terminalCopied = writeOsc52(text);
  let commands: Array<[string, string[]]> = [];
  if (process.platform === "darwin") {
    const escaped = text.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
    commands = [["osascript", ["-e", `set the clipboard to "${escaped}"`]]];
  } else if (process.platform === "linux") {
    commands = [
      ["wl-copy", []],
      ["xclip", ["-selection", "clipboard"]],
      ["xsel", ["--clipboard", "--input"]],
    ];
  } else if (process.platform === "win32") {
    commands = [["powershell.exe", ["-NonInteractive", "-NoProfile", "-Command", "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())"]]];
  }
  for (const [command, args] of commands) {
    if (await copyWithCommand(command, args, text)) return true;
  }
  return terminalCopied;
}

export const System1RouterTui: TuiPlugin = async (api) => {
  const directory = consentDirectory();
  const instanceId = randomUUID();
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  await fs.chmod(directory, 0o700).catch(() => {});

  let activeId: string | undefined;
  let scanning = false;
  let disposed = false;
  let dialogRevision = 0;

  const heartbeatPath = path.join(directory, "heartbeat.json");
  const updateHeartbeat = async () => writeJsonAtomic(heartbeatPath, { instanceId, at: Date.now() }).catch(() => {});
  await updateHeartbeat();

  const copyPath = async (filename: string) => {
    if (await copyToClipboard(filename)) {
      api.ui.toast({ title: "System1Router", message: `Copied path to clipboard: ${filename}`, variant: "success" });
      return;
    }
    api.ui.toast({ title: "System1Router", message: `Could not access the clipboard. Path: ${filename}`, variant: "warning" });
  };

  const offerCopyPath = async (title: string, filename: string, createIfMissing = false) => {
    try {
      if (createIfMissing) {
        await fs.mkdir(path.dirname(filename), { recursive: true, mode: 0o700 });
        const handle = await fs.open(filename, "a", 0o600);
        await handle.chmod(0o600).catch(() => {});
        await handle.close();
      }
    } catch (error) {
      alert(api, title, `Path: ${filename}\n\nCould not access this file: ${String(error)}`);
      return;
    }

    api.ui.dialog.replace(
      () => {
        const props = {
          title,
          options: [
            { title: "Copy", value: "copy", description: filename },
            { title: "Cancel", value: "cancel" },
          ],
          skipFilter: true,
          onSelect: (option: { value: unknown }) => {
            api.ui.dialog.clear();
            if (option.value === "copy") void copyPath(filename);
          },
        };
        // OpenCode 1.18 supports this UI prop; it is missing from the plugin's public type.
        Object.assign(props, { renderFilter: false });
        return api.ui.DialogSelect(props);
      },
      () => {},
    );
  };

  const copyConfigPath = () => offerCopyPath("System1Router config path", configFilePath());
  const copyLogPath = () => offerCopyPath("System1Router provider log path", providerLogPath(), true);

  const toggleRouter = async () => {
    if (process.env.SYSTEM1ROUTER_OPENCODE_DISABLED === "1") {
      api.ui.toast({ title: "System1Router", message: "Routing is forced off by SYSTEM1ROUTER_OPENCODE_DISABLED=1.", variant: "warning" });
      return;
    }
    const filename = configFilePath();
    let enabled: boolean;
    try {
      if (filename.endsWith(".toml")) {
        const contents = await fs.readFile(filename, "utf8");
        enabled = !(await routerEnabled());
        const section = /(^|\n)(\s*\[routing\][^\n]*\n)([\s\S]*?)(?=\n\s*\[|$)/;
        const updated = section.test(contents)
          ? contents.replace(section, (_match, prefix: string, header: string, body: string) =>
              `${prefix}${header}${/^\s*enabled\s*=/m.test(body) ? body.replace(/^\s*enabled\s*=.*$/m, `enabled = ${enabled}`) : `enabled = ${enabled}\n${body}`}`)
          : `${contents.trimEnd()}\n\n[routing]\nenabled = ${enabled}\n`;
        await writeConfigTextAtomic(filename, updated);
      } else {
        const config = JSON.parse(await fs.readFile(filename, "utf8")) as { routing?: Record<string, unknown> };
        if (!config || typeof config !== "object" || Array.isArray(config)) throw new Error("config root must be a JSON object");
        enabled = config.routing?.enabled !== true;
        config.routing = { ...config.routing, enabled };
        await writeConfigAtomic(filename, config);
      }
    } catch (error) {
      api.ui.toast({ title: "System1Router", message: `Could not toggle routing in ${filename}: ${String(error)}`, variant: "error" });
      return;
    }
    if (!enabled && activeId) {
      try {
        const request = JSON.parse(await fs.readFile(path.join(directory, `${activeId}.request.json`), "utf8")) as ConsentRequest;
        activeId = undefined;
        await writeJsonAtomic(path.join(directory, `${request.id}.response.json`), { id: request.id, accepted: false, answeredAt: Date.now() });
        api.ui.dialog.clear();
      } catch {
        activeId = undefined;
        api.ui.dialog.clear();
      }
    }
    api.ui.toast({
      title: `System1Router ${enabled ? "ON" : "OFF"}`,
      message: enabled ? "New prompts will be classified." : "New prompts will keep the current model.",
      variant: enabled ? "success" : "info",
    });
  };

  const toggleVerbose = async () => {
    if (process.env.SYSTEM1ROUTER_VERBOSE !== undefined) {
      api.ui.toast({ title: "System1Router verbose", message: "SYSTEM1ROUTER_VERBOSE is set and overrides this toggle. Unset it to control verbose mode here.", variant: "warning" });
      return;
    }
    const enabled = !(await verboseEnabled());
    try {
      await writeJsonAtomic(verboseStatePath(), { enabled, changedAt: Date.now() });
    } catch (error) {
      api.ui.toast({ title: "System1Router verbose", message: `Could not save verbose setting: ${String(error)}`, variant: "error" });
      return;
    }
    api.ui.toast({
      title: `System1Router verbose ${enabled ? "ON" : "OFF"}`,
      message: enabled ? `Redacted traces appear with each prompt. Full log: ${providerLogPath()}` : "Prompt traces and provider-body logging are off.",
      variant: enabled ? "success" : "info",
    });
  };

  const runControl = async (action: string) => {
    api.ui.dialog.clear();
    if (action === "verbose") {
      await toggleVerbose();
      return;
    }
    if (action === "toggle-router") {
      await toggleRouter();
      return;
    }
    if (action === "config") {
      await copyConfigPath();
      return;
    }
    if (action === "logs") {
      await copyLogPath();
      return;
    }
  };

  const openControlPanel = async () => {
    const [enabled, verbose] = await Promise.all([routerEnabled(), verboseEnabled()]);
    api.ui.dialog.replace(
      () =>
        api.ui.DialogSelect({
          title: "System1Router",
          options: [
            { title: `Router: ${enabled ? "ON" : "OFF"}`, value: "toggle-router", description: "Toggle routing.enabled in the active config" },
            { title: `Verbose traces: ${verbose ? "ON" : "OFF"}`, value: "verbose", description: "Show redacted Jev exchanges with each prompt" },
            { title: "Copy config path…", value: "config", description: configFilePath() },
            { title: "Copy provider log path…", value: "logs", description: providerLogPath() },
          ],
          onSelect: (option) => void runControl(String(option.value)),
        }),
      () => {},
    );
  };

  const showStats = async () => {
    const stats = await readStats();
    const models = Object.entries(stats.models).sort((a, b) => b[1] - a[1]);
    const breakdown = models.length ? models.map(([name, count]) => `  ${name}: ${count}`).join("\n") : "  No calls recorded yet";
    const percentage = stats.suggestions ? `${(100 * stats.accepted / stats.suggestions).toFixed(1)}%` : "—";
    alert(api, "System1Router stats", `Calls: ${stats.calls}\n\nCalls by model:\n${breakdown}\n\nSuggestions answered: ${stats.suggestions}\nAccepted: ${stats.accepted} (${percentage})\nDeclined: ${stats.declined}`);
  };

  const unregisterCommand = api.command.register(() => [
    {
      title: "Toggle System1Router routing",
      value: "s1router-toggle",
      description: "Toggle routing.enabled in the active config",
      category: "System1Router",
      slash: { name: "s1router_toogle", aliases: ["s1router_toggle"] },
      onSelect: () => void toggleRouter(),
    },
    {
      title: "System1Router configuration",
      value: "s1router-config",
      description: "Routing, verbose traces, config and log paths",
      category: "System1Router",
      slash: { name: "s1router_config" },
      onSelect: () => void openControlPanel(),
    },
    {
      title: "System1Router stats",
      value: "s1router-stats",
      description: "Calls per model and suggestion acceptance",
      category: "System1Router",
      slash: { name: "s1router_stats" },
      onSelect: () => void showStats(),
    },
  ]);

  const respond = async (request: ConsentRequest, accepted: boolean) => {
    if (activeId !== request.id) return;
    activeId = undefined;
    await writeJsonAtomic(path.join(directory, `${request.id}.response.json`), {
      id: request.id,
      accepted,
      answeredAt: Date.now(),
    }).catch(() => {});
    api.ui.dialog.clear();
    void scan();
  };

  function showModelChoices(request: ConsentRequest): void {
    if (disposed || activeId !== request.id) return;
    const seconds = Math.max(0, Math.ceil((request.expiresAt - Date.now()) / 1000));
    if (seconds === 0) { void respond(request, false); return; }
    const revision = ++dialogRevision;
    const remaining = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
    api.ui.dialog.replace(
      () => api.ui.DialogConfirm({
        title: `Use ${formatModel(request.recommendation)}?`,
        message: `Current: ${formatModel(request.current)}\n${request.recommendation.reason ? `${request.recommendation.reason}\n` : ""}${remaining} to decide; timeout keeps the current model.`,
        onConfirm: () => void respond(request, Date.now() < request.expiresAt),
        onCancel: () => void respond(request, false),
      }),
      () => { if (revision === dialogRevision) void respond(request, false); },
    );
  }

  async function scan(): Promise<void> {
    if (disposed || scanning || activeId) return;
    scanning = true;
    try {
      const names = (await fs.readdir(directory)).filter((name) => name.endsWith(".request.json"));
      const requests: ConsentRequest[] = [];
      for (const name of names) {
        const filename = path.join(directory, name);
        try {
          const request = JSON.parse(await fs.readFile(filename, "utf8")) as ConsentRequest;
          const responsePath = path.join(directory, `${request.id}.response.json`);
          if (await fs.access(responsePath).then(() => true).catch(() => false)) {
            await fs.rm(filename, { force: true });
            continue;
          }
          if (!Number.isFinite(request.expiresAt) || request.expiresAt <= Date.now()) {
            await fs.rm(filename, { force: true });
            continue;
          }
          if (request.id && request.recommendation?.model && request.recommendation?.provider) requests.push(request);
        } catch {
          // Ignore partially written, malformed, or already-removed requests.
        }
      }
      requests.sort((a, b) => a.createdAt - b.createdAt);
      const request = requests[0];
      if (!request) return;

      activeId = request.id;
      showModelChoices(request);
    } catch {
      // If the dialog could not open, do not leave the queue permanently blocked.
      activeId = undefined;
    } finally {
      scanning = false;
    }
  }

  const heartbeatTimer = setInterval(() => void updateHeartbeat(), 1000);
  const requestTimer = setInterval(() => void scan(), 200);
  const expiryTimer = setInterval(() => {
    if (!activeId) return;
    const id = activeId;
    const requestPath = path.join(directory, `${id}.request.json`);
    void fs.readFile(requestPath, "utf8").then((text) => {
      const request = JSON.parse(text) as ConsentRequest;
      if (request.expiresAt <= Date.now()) void respond(request, false);
    }).catch(() => {
      if (activeId !== id) return;
      activeId = undefined;
      api.ui.dialog.clear();
    });
  }, 1000);

  api.lifecycle.onDispose(async () => {
    disposed = true;
    unregisterCommand();
    clearInterval(heartbeatTimer);
    clearInterval(requestTimer);
    clearInterval(expiryTimer);
    if (activeId) {
      try {
        const request = JSON.parse(await fs.readFile(path.join(directory, `${activeId}.request.json`), "utf8")) as ConsentRequest;
        await respond(request, false);
      } catch {
        api.ui.dialog.clear();
      }
    }
    try {
      const heartbeat = JSON.parse(await fs.readFile(heartbeatPath, "utf8")) as { instanceId?: string };
      if (heartbeat.instanceId === instanceId) await fs.rm(heartbeatPath, { force: true });
    } catch {
      // Heartbeat already removed.
    }
  });
};

export default { id: "system1router-tui", tui: System1RouterTui };
