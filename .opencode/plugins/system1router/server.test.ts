import { test, expect } from "bun:test";
import { promises as fs } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { System1RouterServer } from "./server";
import { consentDirectory, forgetPreferences, readStats, rememberChoice } from "./shared";

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
      routing: { enabled: true }, timeouts: { consentMs: 500 }, providers: { selected: "jev", jev: { endpoint: "http://localhost", credentialEnv: null } },
      rules: [{ id: "simple", routingClass: "code_simple", target: { provider: "p", model: "suggested", variant: "high" }, reason: "Small change" }],
    }));
    await fs.mkdir(consentDirectory(), { recursive: true });

    const history = [
      { info: { id: "prior-user", role: "user" }, parts: [
        { type: "text", text: "We are implementing the parser." },
        { type: "file", filename: path.join(root, "src", "parser.rs"), mime: "text/plain", url: "file:///parser.rs" },
      ] },
      { info: { id: "prior-assistant", role: "assistant", tokens: { input: 1200, output: 50, cache: { read: 3000, write: 0 } } }, parts: [
        { type: "text", text: "The parser uses recursive descent." },
        { type: "tool", tool: "read", state: { status: "completed", input: { filePath: path.join(root, "src", "parser.rs") }, output: "parse_expression reads the left-hand token." } },
      ] },
      { info: { id: "first", role: "user" }, parts: [{ type: "text", text: "Make a small change" }] },
    ];
    const toasts: string[] = [];
    let catalogCalls = 0;
    const plugin = await System1RouterServer({
      directory: root,
      client: {
        session: { messages: async () => ({ data: history }) },
        provider: { list: async () => { catalogCalls++; return { data: { connected: ["p"], all: [{ id: "p", models: {
          current: { id: "current", variants: { "current-variant": {} } },
          suggested: { id: "suggested", variants: { high: {}, low: { disabled: true } } },
        } }] } }; } },
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
              const requestPath = path.join(consentDirectory(), name);
              const request = JSON.parse(await fs.readFile(requestPath, "utf8")) as {
                id: string; expiresAt: number; createdAt: number; recommendation: { routingClass?: string };
              };
              expect(request.recommendation.routingClass).toBe("code_simple");
              observedTimeouts.push(request.expiresAt - request.createdAt);
              if (id === "first") {
                await fs.writeFile(requestPath, JSON.stringify({ ...request, expiresAt: Date.now() + 1500 }));
                await Bun.sleep(600);
              }
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
    expect(acceptedDetails?.text).toContain("Current message:\nMake a small change");
    expect(acceptedDetails?.text).toContain("We are implementing the parser.");
    const verboseInput = acceptedDetails?.text ?? "";
    expect(verboseInput.indexOf("Recent conversation:")).toBeLessThan(verboseInput.indexOf("Current message:"));
    expect(acceptedDetails?.text).toContain("The parser uses recursive descent.");
    expect(acceptedDetails?.text).toContain("Tool read (completed)");
    expect(acceptedDetails?.text).toContain('Input: {"filePath":"src/parser.rs"}');
    expect(acceptedDetails?.text).toContain("parse_expression reads the left-hand token.");
    expect(acceptedDetails?.text).toContain("src/parser.rs");
    expect(acceptedDetails?.text).not.toContain(root);
    const jevRequest = acceptedDetails?.text?.match(/Jev request:\n([\s\S]*?)\n\nJev response:/)?.[1];
    expect(jevRequest).toBeDefined();
    const providerInput = JSON.parse(jevRequest ?? "{}") as { state?: string };
    const currentRequestMarker = "\n\nCurrent request: ";
    const currentRequestStart = providerInput.state?.lastIndexOf(currentRequestMarker) ?? -1;
    expect(currentRequestStart).toBeGreaterThanOrEqual(0);
    expect(providerInput.state?.slice(currentRequestStart + currentRequestMarker.length)).toBe("Make a small change");
    const routedContext = providerInput.state!.slice(0, currentRequestStart);
    expect(routedContext).toContain("Recent conversation:\nUser:\nWe are implementing the parser.");
    expect(routedContext).not.toContain("Make a small change");
    expect(routedContext).not.toContain("null");
    expect(routedContext).toContain('Input: {"filePath":"src/parser.rs"}');
    expect(routedContext).toContain("Files in play: src/parser.rs");
    expect(routedContext).toContain("Tools used: read");
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
    expect(catalogCalls).toBe(1); // cached across turns

    // "Never switch" answers future identical suggestions without a dialog.
    await rememberChoice({ provider: "p", model: "current", variant: "current-variant" }, { provider: "p", model: "suggested", variant: "high" }, false);
    await fs.rm(path.join(consentDirectory(), "heartbeat.json"), { force: true });
    const rememberedOutput = await send("remembered");
    expect(rememberedOutput.message.model.modelID).toBe("current");
    expect(rememberedOutput.parts.some((part) => part.text === "╭─ → S1R / code_simple\n│  ◇ current · current-variant\n╰─\n")).toBe(true);
    await rememberChoice({ provider: "p", model: "current", variant: "current-variant" }, { provider: "p", model: "suggested", variant: "high" }, true);
    const rememberedAccept = await send("remembered-accept");
    expect(rememberedAccept.message.model.modelID).toBe("suggested");
    await forgetPreferences();

    const stats = await readStats();
    expect(observedTimeouts).toEqual([500, 500]);
    expect(stats.calls).toBe(5);
    expect(stats.models).toEqual({ "p/suggested": 2, "p/current": 3 });
    expect([stats.suggestions, stats.accepted, stats.declined]).toEqual([4, 2, 2]);
    const events = (await fs.readFile(process.env.SYSTEM1ROUTER_TELEMETRY!, "utf8")).trim().split("\n").map((line) => JSON.parse(line) as { policyOutcome?: string });
    expect(events.some((event) => event.policyOutcome === "declined")).toBe(false); // the core no longer guesses the user's answer

    const alreadySelected = { message: { id: "same", model: { providerID: "p", modelID: "suggested", variant: "high" } }, parts: [{ type: "text", text: "Another simple task" }] };
    await plugin["chat.message"]?.({ sessionID: "s", messageID: "same", model: { providerID: "p", modelID: "suggested" } } as never, alreadySelected as never);
    // Every rule already points at the current model, so the provider is not called at all.
    expect(alreadySelected.parts.some((part) => part.text === "╭─ → S1R\n│  ◇ suggested · high\n╰─\n")).toBe(true);

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
