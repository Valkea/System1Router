import { test, expect } from "bun:test";
import { promises as fs } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { System1RouterServer } from "./server";
import { consentDirectory, readStats } from "./shared";

const binary = path.resolve(import.meta.dir, "../../../target/debug/system1router");

test("routes the same turn and counts actual acceptances, declines and missing UI", async () => {
  const root = await fs.mkdtemp(path.join(tmpdir(), "system1router-test-"));
  const before = Object.fromEntries(["XDG_STATE_HOME", "SYSTEM1ROUTER_CONFIG", "SYSTEM1ROUTER_BIN", "SYSTEM1ROUTER_JEV_RESPONSE", "SYSTEM1ROUTER_VERBOSE", "SYSTEM1ROUTER_TELEMETRY"].map((key) => [key, process.env[key]]));
  try {
    process.env.XDG_STATE_HOME = path.join(root, "state");
    process.env.SYSTEM1ROUTER_CONFIG = path.join(root, "config.json");
    process.env.SYSTEM1ROUTER_BIN = binary;
    process.env.SYSTEM1ROUTER_VERBOSE = "1";
    process.env.SYSTEM1ROUTER_TELEMETRY = path.join(root, "events.jsonl");
    process.env.SYSTEM1ROUTER_JEV_RESPONSE = JSON.stringify({
      routingClass: "code_simple", taskType: "code-generation", contextSignals: [],
      confidence: { routingClass: 0.95, taskType: 0.9, context: 0.8 },
      providerMetadata: { provider: "jev", version: "test" }, correlationId: "fixture",
    });
    await fs.writeFile(process.env.SYSTEM1ROUTER_CONFIG, JSON.stringify({
      routing: { enabled: true }, timeouts: { consentMs: 120000 }, providers: { selected: "jev", jev: { endpoint: "http://localhost", credentialEnv: null } },
      rules: [{ id: "simple", routingClass: "code_simple", target: { provider: "p", model: "suggested", variant: "high" }, reason: "Small change" }],
    }));
    await fs.mkdir(consentDirectory(), { recursive: true });

    const toasts: string[] = [];
    const plugin = await System1RouterServer({
      directory: root,
      client: {
        provider: { list: async () => ({ data: { connected: ["p"], all: [{ id: "p", models: { current: { id: "current" }, suggested: { id: "suggested" } } }] } }) },
        tui: { showToast: async ({ body }: { body: { message: string } }) => { toasts.push(body.message); } },
      },
    } as never);
    const observedTimeouts: number[] = [];

    async function send(id: string, choice?: boolean) {
      const output = { message: { id, model: { providerID: "p", modelID: "current", variant: "current-variant" } }, parts: [{ id: "prt_01ARZ3NDEKTSV4RRFFQ69G5FAV", type: "text", text: "Make a small change" }] };
      if (typeof choice === "boolean") {
        await fs.writeFile(path.join(consentDirectory(), "heartbeat.json"), JSON.stringify({ at: Date.now() }));
        const answer = (async () => {
          for (let attempt = 0; attempt < 100; attempt++) {
            const names = await fs.readdir(consentDirectory());
            const name = names.find((item) => item.endsWith(".request.json"));
            if (name) {
              const request = JSON.parse(await fs.readFile(path.join(consentDirectory(), name), "utf8")) as { id: string; expiresAt: number; createdAt: number };
              observedTimeouts.push(request.expiresAt - request.createdAt);
              await fs.writeFile(path.join(consentDirectory(), `${request.id}.response.json`), JSON.stringify({ id: request.id, accepted: choice }));
              return;
            }
            await Bun.sleep(20);
          }
          throw new Error("confirmation request never arrived");
        })();
        await plugin["chat.message"]?.({ sessionID: "s", messageID: id, model: { providerID: "p", modelID: "current" } } as never, output as never);
        await answer;
      } else {
        await fs.rm(path.join(consentDirectory(), "heartbeat.json"), { force: true });
        await plugin["chat.message"]?.({ sessionID: "s", messageID: id, model: { providerID: "p", modelID: "current" } } as never, output as never);
      }
      return output;
    }

    const accepted = await send("first", true);
    expect(accepted.message.model.modelID).toBe("suggested");
    expect(accepted.message.model.variant).toBe("high");
    const acceptedParts = accepted.parts as Array<{
      id: string;
      type: string;
      text?: string;
      ignored?: boolean;
      time?: { start: number; end?: number };
      metadata?: { system1router?: string };
    }>;
    const acceptedStatus = acceptedParts.find((part) => part.metadata?.system1router === "routing-status");
    const acceptedDetails = acceptedParts.find((part) => part.metadata?.system1router === "verbose-details");
    expect(acceptedStatus?.text).toBe("╭─ → S1R / code_simple\n│  ◇ suggested · high\n╰─\n");
    expect(acceptedStatus?.id).toMatch(/^prt_0{26}/);
    expect(acceptedStatus?.ignored).toBe(true);
    expect(acceptedParts[1]?.text).toBe("Make a small change");
    expect([...acceptedParts].sort((a, b) => a.id.localeCompare(b.id))[0]).toBe(acceptedStatus);
    expect(acceptedParts.some((part) => part.metadata?.system1router === "verbose-decision")).toBe(false);
    expect(acceptedDetails?.type).toBe("reasoning");
    expect(acceptedDetails?.text).toContain("Input:\nMake a small change");
    expect(acceptedDetails?.text).toContain("Output:\n");
    expect(acceptedDetails?.text).toContain("Jev request:");
    expect(acceptedDetails?.text).toContain("Jev response:");
    expect(acceptedDetails?.ignored).toBe(true);
    expect(acceptedDetails?.time?.end).toBeDefined();
    expect(acceptedParts.indexOf(acceptedStatus!)).toBeLessThan(acceptedParts.indexOf(acceptedDetails!));
    const declined = await send("second", false);
    expect(declined.message.model.modelID).toBe("current");
    expect(declined.message.model.variant).toBe("current-variant");
    expect((declined.parts as typeof acceptedParts).some((part) => part.metadata?.system1router === "routing-status")).toBe(false);
    expect((declined.parts as typeof acceptedParts).some((part) => part.metadata?.system1router === "verbose-details")).toBe(true);
    const missingUi = await send("third");
    expect(missingUi.message.model.modelID).toBe("current");
    expect(toasts).toContain("Routing this request…");
    const stats = await readStats();
    expect(observedTimeouts).toEqual([120000, 120000]);
    expect(stats.calls).toBe(3);
    expect(stats.models).toEqual({ "p/suggested": 1, "p/current": 2 });
    expect([stats.suggestions, stats.accepted, stats.declined]).toEqual([2, 1, 1]);

    const alreadySelected = { message: { id: "same", model: { providerID: "p", modelID: "suggested", variant: "high" } }, parts: [{ type: "text", text: "Another simple task" }] };
    await plugin["chat.message"]?.({ sessionID: "s", messageID: "same", model: { providerID: "p", modelID: "suggested" } } as never, alreadySelected as never);
    expect(alreadySelected.parts.some((part) => part.text === "╭─ → S1R / code_simple\n│  ◇ suggested · high\n╰─\n")).toBe(true);

    await fs.writeFile(process.env.SYSTEM1ROUTER_CONFIG, JSON.stringify({ routing: { enabled: true, minConfidence: 9 } }));
    const invalidConfig = await send("fourth");
    expect(invalidConfig.message.model.modelID).toBe("current");
    expect(invalidConfig.parts.some((part) => part.text === "╭─ → S1R\n│  ◇ current · current-variant\n╰─\n")).toBe(true);
  } finally {
    for (const [key, value] of Object.entries(before)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
    await fs.rm(root, { recursive: true, force: true });
  }
});
