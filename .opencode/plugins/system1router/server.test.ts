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
      rules: [{ id: "simple", routingClass: "code_simple", target: { provider: "p", model: "suggested" }, reason: "Small change" }],
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
      const output = { message: { id, model: { providerID: "p", modelID: "current" } }, parts: [{ type: "text", text: "Make a small change" }] };
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
    expect(accepted.parts.some((part) => part.text.includes("using p/suggested"))).toBe(true);
    const declined = await send("second", false);
    expect(declined.message.model.modelID).toBe("current");
    const missingUi = await send("third");
    expect(missingUi.message.model.modelID).toBe("current");
    expect(toasts).toContain("Routing this request…");
    expect(accepted.parts.some((part) => part.text.includes("Jev request:") && part.text.includes("Jev response:"))).toBe(true);
    const stats = await readStats();
    expect(observedTimeouts).toEqual([120000, 120000]);
    expect(stats.calls).toBe(3);
    expect(stats.models).toEqual({ "p/suggested": 1, "p/current": 2 });
    expect([stats.suggestions, stats.accepted, stats.declined]).toEqual([2, 1, 1]);

    await fs.writeFile(process.env.SYSTEM1ROUTER_CONFIG, JSON.stringify({ routing: { enabled: true, minConfidence: 9 } }));
    const invalidConfig = await send("fourth");
    expect(invalidConfig.message.model.modelID).toBe("current");
    expect(invalidConfig.parts.some((part) => part.text.includes("keeping p/current"))).toBe(true);
  } finally {
    for (const [key, value] of Object.entries(before)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
    await fs.rm(root, { recursive: true, force: true });
  }
});
