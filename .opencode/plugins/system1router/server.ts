import { randomUUID } from "node:crypto";
import { promises as fs } from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import type { Plugin } from "@opencode-ai/plugin";
import { consentDirectory, consentTimeoutMs, recordUsage, routerEnabled, verboseEnabled, writeJsonAtomic, type ConsentRequest } from "./shared";

type RouterResult = {
  consent?: string;
  providerTrace?: string;
  telemetry?: { routingClass?: string };
  diagnostic?: { code?: string };
  recommendation?: {
    provider?: string;
    model?: string;
    variant?: string;
    reason?: string;
    routingClass?: string;
  };
};

function providerTraceFromStderr(stderr: string): string | undefined {
  const entries = stderr.split(/\r?\n/).flatMap((line) => {
    const match = line.match(/^\[system1router\] Jev (request|response|error): (.+)$/);
    if (!match) return [];
    try {
      return [`Jev ${match[1]}:\n${JSON.stringify(JSON.parse(match[2]!), null, 2)}`];
    } catch {
      return [`Jev ${match[1]}: ${match[2]}`];
    }
  });
  return entries.length ? entries.join("\n\n") : undefined;
}

function appendVerboseTrace<Part extends object>(
  sessionID: string,
  messageID: string,
  output: { parts: Part[] },
  result: RouterResult,
  query: string,
  context: RouterContext,
  startedAt: number,
): void {
  const input = [
    `Current message:\n${query || "(no text input)"}`,
    context.conversationSummary ? `Recent conversation:\n${context.conversationSummary}` : undefined,
    context.tools.length ? `Tools used: ${context.tools.join(", ")}` : undefined,
    context.files.length ? `Files accessed: ${context.files.join(", ")}` : undefined,
  ].filter(Boolean).join("\n\n");
  let text = [
    "System1Router verbose input/output",
    `Input:\n${input}`,
    `Output:\n${result.providerTrace ?? "No Jev exchange was recorded for this request."}`,
  ].join("\n\n");
  const limit = 12_000;
  if (text.length > limit) text = `${text.slice(0, limit)}\n… trace shortened; open the provider log for the full exchange.`;
  output.parts.push({
    id: `prt_${randomUUID().replaceAll("-", "")}`,
    sessionID,
    messageID,
    type: "reasoning",
    text,
    time: { start: startedAt, end: Date.now() },
    ignored: true,
    metadata: { system1router: "verbose-details" },
  } as Part);
}

function routingStatusText(routingClass: string | undefined, model: string, variant?: string): string {
  const source = routingClass ? `S1R / ${routingClass}` : "S1R";
  return `╭─ → ${source}\n│  ◇ ${model}${variant ? ` · ${variant}` : ""}\n╰─\n`;
}

function prependRoutingStatus<Part extends object>(sessionID: string, messageID: string, output: { parts: Part[] }, text: string): void {
  // OpenCode reloads message parts sorted by ID, so give this block a low-sort ID to keep it before the prompt.
  output.parts.unshift({
    id: `prt_${"0".repeat(26)}${randomUUID().replaceAll("-", "")}`,
    sessionID, messageID, type: "text", text,
    synthetic: false, ignored: true,
    metadata: { system1router: "routing-status" },
  } as Part);
}

async function tuiIsAlive(directory: string): Promise<boolean> {
  try {
    const heartbeat = JSON.parse(await fs.readFile(path.join(directory, "heartbeat.json"), "utf8")) as { at?: number };
    return typeof heartbeat.at === "number" && Date.now() - heartbeat.at < 3_000;
  } catch {
    return false;
  }
}

async function requestConsent(
  directory: string,
  current: ConsentRequest["current"],
  recommendation: ConsentRequest["recommendation"],
): Promise<boolean | undefined> {
  if (!(await tuiIsAlive(directory))) return undefined;

  const id = randomUUID();
  const requestPath = path.join(directory, `${id}.request.json`);
  const responsePath = path.join(directory, `${id}.response.json`);
  const createdAt = Date.now();
  let expiresAt = createdAt + await consentTimeoutMs();
  await writeJsonAtomic(requestPath, { id, createdAt, expiresAt, current, recommendation } satisfies ConsentRequest);

  try {
    while (true) {
      try {
        const request = JSON.parse(await fs.readFile(requestPath, "utf8")) as { id?: string; expiresAt?: number };
        if (request.id === id && Number.isSafeInteger(request.expiresAt) && request.expiresAt! > expiresAt) {
          expiresAt = request.expiresAt!;
        }
      } catch {
        // The TUI may be replacing the request atomically while extending its deadline.
      }
      try {
        const response = JSON.parse(await fs.readFile(responsePath, "utf8")) as { id?: string; accepted?: boolean };
        if (response.id === id && typeof response.accepted === "boolean") return response.accepted;
      } catch {
        // The TUI plugin writes an atomic response file after the user chooses.
      }
      if (Date.now() >= expiresAt) return false;
      if (!(await tuiIsAlive(directory))) return undefined;
      await new Promise((resolve) => setTimeout(resolve, 120));
    }
  } finally {
    await Promise.all([fs.rm(requestPath, { force: true }), fs.rm(responsePath, { force: true })]);
  }
}

function textFromParts(parts: unknown[]): string {
  return parts
    .map((part) => {
      if (!part || typeof part !== "object") return "";
      const value = part as { type?: string; text?: string; ignored?: boolean; synthetic?: boolean };
      return value.type === "text" && !value.ignored && !value.synthetic ? value.text ?? "" : "";
    })
    .filter(Boolean)
    .join("\n");
}

type RecordValue = Record<string, unknown>;
type SessionMessage = { info?: RecordValue; parts?: unknown[] };
type RouterContext = {
  conversationSummary?: string;
  files: string[];
  tools: string[];
  attachments: RecordValue[];
  toolCalls: RecordValue[];
};

function asRecord(value: unknown): RecordValue | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as RecordValue : undefined;
}

function clipText(value: string, limit: number): string {
  const characters = Array.from(value);
  return characters.length > limit ? `${characters.slice(0, limit).join("")}…[truncated]` : value;
}

function keepRecentText(value: string, limit: number): string {
  const characters = Array.from(value);
  if (characters.length <= limit) return value;
  return `[Earlier conversation omitted]\n${characters.slice(-(limit - 30)).join("")}`;
}

function displayFilePath(value: string, directory: string): string {
  let normalized = value.trim().replaceAll("\\", "/");
  if (!normalized) return "";
  if (normalized.startsWith("file://")) {
    try { normalized = decodeURIComponent(new URL(normalized).pathname); } catch { /* Keep the original path */ }
  }
  const root = directory.replaceAll("\\", "/").replace(/\/+$/, "");
  if (root && normalized.startsWith(`${root}/`)) return normalized.slice(root.length + 1);
  if (normalized === root) return ".";
  if (normalized.startsWith("/") || /^[A-Za-z]:\//.test(normalized)) return normalized.split("/").pop() ?? "";
  return normalized.replace(/^\.\//, "");
}

function recentUnique(values: string[], limit: number): string[] {
  const output: string[] = [];
  for (let index = values.length - 1; index >= 0 && output.length < limit; index--) {
    const value = values[index];
    if (value && !output.includes(value)) output.push(value);
  }
  return output.reverse();
}

function safeToolInput(value: unknown, directory: string, key = ""): unknown {
  if (/token|secret|password|authorization|credential|api.?key/i.test(key)) return "[REDACTED]";
  if (typeof value === "string") {
    if (/^(filePath|filename|path|file|workdir)$/i.test(key)) return displayFilePath(value, directory);
    return clipText(value.replaceAll(directory, "."), 240);
  }
  if (Array.isArray(value)) return value.slice(0, 8).map((item) => safeToolInput(item, directory, key));
  const object = asRecord(value);
  if (object) {
    return Object.fromEntries(Object.entries(object).slice(0, 16).map(([name, item]) => [name, safeToolInput(item, directory, name)]));
  }
  return value;
}

function collectFilePaths(value: unknown, directory: string, output: string[], key = ""): void {
  if (typeof value === "string") {
    if (/^(filePath|filename|path|file|files|loaded|workdir)$/i.test(key)) {
      const filename = displayFilePath(value, directory);
      if (filename) output.push(filename);
    }
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) collectFilePaths(item, directory, output, key);
    return;
  }
  const object = asRecord(value);
  if (object) for (const [name, item] of Object.entries(object)) collectFilePaths(item, directory, output, name);
}

function addFilePart(part: RecordValue, directory: string, files: string[], attachments: RecordValue[]): string | undefined {
  const source = asRecord(part.source);
  const filename = typeof part.filename === "string" ? part.filename : typeof source?.path === "string" ? source.path : undefined;
  if (!filename) return undefined;
  const name = displayFilePath(filename, directory);
  if (!name) return undefined;
  files.push(name);
  const mime = typeof part.mime === "string" ? part.mime : undefined;
  attachments.push({ filename: name, ...(mime ? { mime } : {}) });
  const sourceText = asRecord(source?.text)?.value;
  return `[File: ${name}${mime ? ` (${mime})` : ""}]${typeof sourceText === "string" ? `\n${clipText(sourceText, 400)}` : ""}`;
}

function projectConversationContext(
  history: SessionMessage[],
  currentMessageID: string,
  currentParts: unknown[],
  directory: string,
): RouterContext {
  const blocks: string[] = [];
  const files: string[] = [];
  const tools: string[] = [];
  const attachments: RecordValue[] = [];
  const toolCalls: RecordValue[] = [];
  const currentFiles: string[] = [];

  for (const rawPart of currentParts) {
    const part = asRecord(rawPart);
    if (part?.type === "file") {
      const label = addFilePart(part, directory, files, attachments);
      if (label) currentFiles.push(label);
    }
  }

  for (const message of history.slice(-100)) {
    const info = asRecord(message.info);
    const messageID = typeof info?.id === "string" ? info.id : undefined;
    if (messageID === currentMessageID) continue;
    const role = info?.role === "user" ? "User" : info?.role === "assistant" ? "Assistant" : undefined;
    if (!role) continue;

    const messageParts: string[] = [];
    for (const rawPart of message.parts ?? []) {
      const part = asRecord(rawPart);
      if (!part) continue;
      const metadata = asRecord(part.metadata);
      if (metadata?.system1router) continue;

      if (part.type === "text" && !(role === "User" && part.ignored === true) && typeof part.text === "string" && part.text) {
        messageParts.push(clipText(part.text.replaceAll(directory, "."), 900));
        continue;
      }
      if (part.type === "file") {
        const label = addFilePart(part, directory, files, attachments);
        if (label) messageParts.push(label);
        continue;
      }
      if (part.type === "reasoning" && typeof part.text === "string" && part.text) {
        messageParts.push(`Reasoning:\n${clipText(part.text.replaceAll(directory, "."), 500)}`);
        continue;
      }
      if (part.type === "tool") {
        const tool = typeof part.tool === "string" ? part.tool : "tool";
        const state = asRecord(part.state) ?? {};
        const status = typeof state.status === "string" ? state.status : "unknown";
        const input = asRecord(state.input) ?? {};
        const stateMetadata = asRecord(state.metadata) ?? {};
        const stateTime = asRecord(state.time) ?? {};
        tools.push(tool);
        collectFilePaths(input, directory, files);
        collectFilePaths(stateMetadata.loaded, directory, files, "loaded");
        for (const attachment of Array.isArray(state.attachments) ? state.attachments : []) {
          const label = addFilePart(asRecord(attachment) ?? {}, directory, files, attachments);
          if (label) messageParts.push(label);
        }

        const inputSummary = clipText(JSON.stringify(safeToolInput(input, directory)), 180);
        toolCalls.push({ tool, status, input: inputSummary });

        let output = "";
        if (status === "completed") {
          output = typeof stateTime.compacted === "number" ? "[Old tool result content cleared]" : typeof state.output === "string" ? state.output : "";
        } else if (status === "error") {
          output = typeof state.error === "string" ? state.error : "[Tool failed]";
        } else {
          output = `[Tool ${status}]`;
        }
        const detail = clipText(output.replaceAll(directory, "."), 700);
        messageParts.push(`Tool ${tool} (${status})\nInput: ${inputSummary}${detail ? `\nOutput:\n${detail}` : ""}`);
        continue;
      }
      if (part.type === "subtask") {
        const agent = typeof part.agent === "string" ? part.agent : "agent";
        tools.push(agent);
        const description = typeof part.description === "string" ? part.description : "delegated task";
        const prompt = typeof part.prompt === "string" ? clipText(part.prompt, 300) : "";
        messageParts.push(`Delegated to ${agent}: ${description}${prompt ? `\n${prompt}` : ""}`);
      }
    }
    if (messageParts.length) blocks.push(`${role}:\n${messageParts.join("\n")}`);
  }

  if (currentFiles.length) blocks.push(`Current attachments:\n${currentFiles.join("\n")}`);
  const summary = keepRecentText(blocks.join("\n\n"), 3_000);
  const recentCalls = toolCalls.slice(-5);
  const uniqueAttachments = Array.from(new Map(attachments.map((item) => [`${item.filename}:${item.mime ?? ""}`, item])).values()).slice(-16);

  return {
    ...(summary ? { conversationSummary: summary } : {}),
    files: recentUnique(files, 8).map((file) => clipText(file, 64)),
    tools: recentUnique(tools, 10).map((tool) => clipText(tool, 32)),
    attachments: uniqueAttachments,
    toolCalls: recentCalls,
  };
}

async function sessionHistory(client: Parameters<Plugin>[0]["client"], sessionID: string, directory: string): Promise<SessionMessage[]> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const response = await Promise.race([
      client.session.messages({ path: { id: sessionID }, query: { directory, limit: 100 } }),
      new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error("session history timeout")), 1_000); }),
    ]);
    return Array.isArray(response.data) ? response.data as SessionMessage[] : [];
  } catch {
    return [];
  } finally {
    if (timer) clearTimeout(timer);
  }
}

function invoke(binary: string, payload: unknown, verbose: boolean, timeoutMs = 10_000): Promise<RouterResult> {
  return new Promise((resolve) => {
    const environmentVerbose = process.env.SYSTEM1ROUTER_VERBOSE;
    const child = spawn(binary, [], {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, SYSTEM1ROUTER_VERBOSE: environmentVerbose === undefined ? (verbose ? "1" : "0") : environmentVerbose },
    });
    const output: Uint8Array[] = [];
    const errors: Uint8Array[] = [];
    const timer = setTimeout(() => {
      child.kill();
      resolve({ diagnostic: { code: "timeout" } });
    }, timeoutMs);
    child.stdout.on("data", (chunk: Buffer) => output.push(new Uint8Array(chunk)));
    child.stderr.on("data", (chunk: Buffer) => {
      errors.push(new Uint8Array(chunk));
    });
    child.on("error", () => {
      clearTimeout(timer);
      resolve({ diagnostic: { code: "startup_failure" } });
    });
    child.on("close", () => {
      clearTimeout(timer);
      const providerTrace = providerTraceFromStderr(Buffer.concat(errors).toString("utf8"));
      try {
        resolve({ ...(JSON.parse(Buffer.concat(output).toString("utf8")) as RouterResult), ...(providerTrace ? { providerTrace } : {}) });
      } catch {
        resolve({ diagnostic: { code: "malformed_response" }, ...(providerTrace ? { providerTrace } : {}) });
      }
    });
    // The executable may exit before reading stdin (e.g. invalid config).
    // EPIPE should fail this turn open rather than crash the plugin process.
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify(payload));
  });
}

export const System1RouterServer: Plugin = async ({ client, directory }) => {
  const seen = new Set<string>();
  const notified = new Set<string>();
  const binary = process.env.SYSTEM1ROUTER_BIN ?? "system1router";

  const notify = async (key: string, message: string, variant: "info" | "warning" | "error" = "info") => {
    if (notified.has(key)) return;
    notified.add(key);
    try {
      await client.tui.showToast({
        query: { directory },
        body: { title: "System1Router", message, variant, duration: 10000 },
      });
    } catch {
      process.stderr.write(`[system1router] ${message}\n`);
    }
  };

  return {
    "chat.message": async (input, output) => {
      const messageID = input.messageID ?? output.message.id;
      const key = `${input.sessionID}:${messageID}`;
      if (seen.has(key)) return;
      if (seen.size > 2048) seen.clear();
      seen.add(key);

      const userMessage = output.message as typeof output.message & {
        model: { providerID: string; modelID: string; variant?: string };
      };
      const model = input.model ?? userMessage.model;
      const current = model?.providerID && model.modelID ? `${model.providerID}/${model.modelID}` : undefined;
      const currentVariant = userMessage.model.variant ?? input.variant;
      const currentModelName = model?.modelID ?? "OpenCode default";
      const enabled = await routerEnabled();
      if (!enabled) {
        await notify("disabled", "Routing is off. Configure the provider and rules, then enable it with /s1router_config.");
        await recordUsage({ host: "opencode", model: current, policyOutcome: "disabled" }).catch(() => {});
        return;
      }

      // This appears immediately, before the model's Thinking indicator.
      void client.tui.showToast({ query: { directory }, body: {
        title: "System1Router", message: "Routing this request…", variant: "info", duration: 3000,
      } }).catch(() => {});

      const historyPromise = sessionHistory(client, input.sessionID, directory);
      let catalog: unknown[] = [];
      try {
        const providers = await Promise.race([
          client.provider.list({ query: { directory } }),
          new Promise<never>((_, reject) => setTimeout(() => reject(new Error("catalog timeout")), 1500)),
        ]);
        catalog = providers.data?.all.filter((provider) => providers.data?.connected.includes(provider.id)).flatMap((provider) =>
          Object.values(provider.models).map((model) => ({
            provider: provider.id,
            model: model.id,
            variants: [],
          })),
        ) ?? [];
      } catch {
        // A missing catalog prevents an unverified switch, but not the turn.
      }

      const verbose = await verboseEnabled();
      const query = textFromParts(output.parts);
      const history = await historyPromise;
      const context = projectConversationContext(history, messageID, output.parts, directory);
      const routingStartedAt = Date.now();
      const result = await invoke(binary, {
        protocolVersion: "1",
        host: "opencode",
        event: "chat.message",
        sessionId: input.sessionID,
        turnId: messageID,
        cwd: directory,
        request: { query, attachments: context.attachments, tools: context.toolCalls },
        currentModel: model
          ? { provider: model.providerID, model: model.modelID, variant: userMessage.model.variant ?? input.variant, scope: input.sessionID }
          : undefined,
        context: {
          conversationSummary: context.conversationSummary,
          files: context.files,
          tools: context.tools,
        },
        catalog: { models: catalog },
        capabilities: {
          level: "full-override",
          modelMutation: true,
          interactiveConsent: true,
          gateway: false,
          protocolVersion: "1",
        },
        metadata: { plugin: "opencode", sdk: "@opencode-ai/plugin@1.4.3" },
      }, verbose);

      const appendVerboseDetails = () => {
        if (verbose) appendVerboseTrace(input.sessionID, output.message.id, output, result, query, context, routingStartedAt);
      };

      if (result.diagnostic?.code === "disabled") {
        await notify(
          "disabled",
          "Installed, but routing is disabled. Configure a Jev/Laya provider and set routing.enabled=true in ~/.config/system1router/config.json.",
        );
      } else if (["provider-unavailable", "provider-timeout", "provider-error", "malformed-response"].includes(result.diagnostic?.code ?? "")) {
        await notify(
          result.diagnostic?.code ?? "provider-error",
          "The routing provider could not classify this request. It will continue on the current model; run system1router doctor for diagnostics.",
          "warning",
        );
      } else if (result.diagnostic?.code === "startup_failure") {
        await notify("startup_failure", "System1Router could not start. Check that the executable is on PATH or set SYSTEM1ROUTER_BIN.", "error");
      } else if (result.diagnostic?.code === "malformed_response") {
        await notify("malformed_response", "System1Router returned no decision. Check the config with system1router doctor; the current model will be used.", "warning");
      }

      const recommendation = result.recommendation;
      const routingClass = recommendation?.routingClass ?? result.telemetry?.routingClass;
      if (!recommendation?.provider || !recommendation.model) {
        appendVerboseDetails();
        prependRoutingStatus(input.sessionID, output.message.id, output, routingStatusText(routingClass, currentModelName, currentVariant));
        await recordUsage({ host: "opencode", model: current, policyOutcome: result.diagnostic?.code }).catch(() => {});
        return;
      }
      const suggested = `${recommendation.provider}/${recommendation.model}`;
      if (current === suggested && currentVariant === recommendation.variant) {
        appendVerboseDetails();
        prependRoutingStatus(input.sessionID, output.message.id, output, routingStatusText(routingClass, recommendation.model, recommendation.variant));
        await recordUsage({ host: "opencode", model: current, policyOutcome: "same-model" }).catch(() => {});
        return;
      }
      if (result.consent === "keep_current_model") {
        appendVerboseDetails();
        prependRoutingStatus(input.sessionID, output.message.id, output, routingStatusText(routingClass, currentModelName, currentVariant));
        await recordUsage({ host: "opencode", model: current, suggestedModel: suggested, accepted: false }).catch(() => {});
        return;
      }

      const accepted = result.consent === "use_suggested_model"
        ? true
        : await requestConsent(
            consentDirectory(),
            model
              ? {
                  provider: model.providerID,
                  model: model.modelID,
                  variant: userMessage.model.variant ?? input.variant,
                }
              : undefined,
            {
              provider: recommendation.provider,
              model: recommendation.model,
              variant: recommendation.variant,
              routingClass: recommendation.routingClass,
              reason: recommendation.reason,
            },
          );
      if (accepted === undefined) {
        await notify(
          "consent-ui-unavailable",
          "No active OpenCode confirmation dialog is available. Keeping the current model; start the project TUI and retry.",
          "warning",
        );
        appendVerboseDetails();
        prependRoutingStatus(input.sessionID, output.message.id, output, routingStatusText(routingClass, currentModelName, currentVariant));
        await recordUsage({ host: "opencode", model: current, suggestedModel: suggested, policyOutcome: "consent-unavailable" }).catch(() => {});
        return;
      }
      if (!accepted) {
        appendVerboseDetails();
        await recordUsage({ host: "opencode", model: current, suggestedModel: suggested, accepted: false }).catch(() => {});
        return;
      }
      if (!(await routerEnabled())) {
        appendVerboseDetails();
        prependRoutingStatus(input.sessionID, output.message.id, output, routingStatusText(routingClass, currentModelName, currentVariant));
        await recordUsage({ host: "opencode", model: current, suggestedModel: suggested }).catch(() => {});
        return;
      }

      // chat.message fires before OpenCode saves this user message and starts generation.
      // Mutating its model here routes this same request; no duplicate prompt is submitted.
      userMessage.model = {
        providerID: recommendation.provider,
        modelID: recommendation.model,
        ...(recommendation.variant ? { variant: recommendation.variant } : {}),
      };
      appendVerboseDetails();
      prependRoutingStatus(input.sessionID, output.message.id, output, routingStatusText(routingClass, recommendation.model, recommendation.variant));
      await recordUsage({ host: "opencode", model: suggested, suggestedModel: suggested, accepted: true }).catch(() => {});
      await notify(
        `${key}:accepted`,
        `Using ${suggested}${recommendation.variant ? ` (${recommendation.variant})` : ""} for this request.`,
        "info",
      );
    },
  };
};

export default { id: "system1router", server: System1RouterServer };
