import { randomUUID } from "node:crypto";
import { promises as fs } from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import type { Plugin } from "@opencode-ai/plugin";
import { consentDirectory, consentTimeoutMs, recordUsage, routerEnabled, verboseEnabled, writeJsonAtomic, type ConsentRequest } from "./shared";

type RouterResult = {
  outcome?: string;
  consent?: string;
  correlationId?: string;
  providerTrace?: string;
  telemetry?: { routingClass?: string; taskType?: string };
  diagnostic?: { code?: string; message?: string; details?: { providerError?: string } };
  recommendation?: {
    provider?: string;
    model?: string;
    variant?: string;
    reason?: string;
    routingClass?: string;
    taskType?: string;
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
  startedAt: number,
): void {
  let text = [
    "System1Router verbose input/output",
    `Input:\n${query || "(no text input)"}`,
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
  const expiresAt = createdAt + await consentTimeoutMs();
  await writeJsonAtomic(requestPath, { id, createdAt, expiresAt, current, recommendation } satisfies ConsentRequest);

  try {
    while (Date.now() < expiresAt) {
      try {
        const response = JSON.parse(await fs.readFile(responsePath, "utf8")) as { id?: string; accepted?: boolean };
        if (response.id === id && typeof response.accepted === "boolean") return response.accepted;
      } catch {
        // The TUI plugin writes an atomic response file after the user chooses.
      }
      if (!(await tuiIsAlive(directory))) return undefined;
      await new Promise((resolve) => setTimeout(resolve, 120));
    }
    return false;
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

function invoke(binary: string, payload: unknown, verbose: boolean, timeoutMs = 10_000): Promise<RouterResult> {
  return new Promise((resolve) => {
    const environmentVerbose = process.env.SYSTEM1ROUTER_VERBOSE;
    const child = spawn(binary, [], {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, SYSTEM1ROUTER_VERBOSE: environmentVerbose === undefined ? (verbose ? "1" : "0") : environmentVerbose },
    });
    const output: Buffer[] = [];
    const errors: Buffer[] = [];
    const timer = setTimeout(() => {
      child.kill();
      resolve({ outcome: "bypass", diagnostic: { code: "timeout", message: "router invocation timed out" } });
    }, timeoutMs);
    child.stdout.on("data", (chunk: Buffer) => output.push(chunk));
    child.stderr.on("data", (chunk: Buffer) => {
      errors.push(chunk);
    });
    child.on("error", () => {
      clearTimeout(timer);
      resolve({ outcome: "bypass", diagnostic: { code: "startup_failure", message: "system1router executable is unavailable" } });
    });
    child.on("close", () => {
      clearTimeout(timer);
      const providerTrace = providerTraceFromStderr(Buffer.concat(errors).toString("utf8"));
      try {
        resolve({ ...(JSON.parse(Buffer.concat(output).toString("utf8")) as RouterResult), ...(providerTrace ? { providerTrace } : {}) });
      } catch {
        resolve({ outcome: "bypass", diagnostic: { code: "malformed_response", message: "router returned no valid decision" }, ...(providerTrace ? { providerTrace } : {}) });
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
      const routingStartedAt = Date.now();
      const result = await invoke(binary, {
        protocolVersion: "1",
        host: "opencode",
        event: "chat.message",
        sessionId: input.sessionID,
        turnId: messageID,
        cwd: directory,
        request: { query },
        currentModel: model
          ? { provider: model.providerID, model: model.modelID, variant: userMessage.model.variant ?? input.variant, scope: input.sessionID }
          : undefined,
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
        if (verbose) appendVerboseTrace(input.sessionID, output.message.id, output, result, query, routingStartedAt);
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
        "success",
      );
    },
  };
};

export default { id: "system1router", server: System1RouterServer };
