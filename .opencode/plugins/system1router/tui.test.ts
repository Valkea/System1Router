import { test, expect } from "bun:test";
import { promises as fs } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { System1RouterTui } from "./tui";
import { consentDirectory } from "./shared";

test("exposes only the three router commands and uses readable model choices", async () => {
  const root = await fs.mkdtemp(path.join(tmpdir(), "system1router-tui-"));
  const oldState = process.env.XDG_STATE_HOME;
  const oldConfig = process.env.SYSTEM1ROUTER_CONFIG;
  process.env.XDG_STATE_HOME = path.join(root, "state");
  process.env.SYSTEM1ROUTER_CONFIG = path.join(root, "config.json");
  let dispose: () => Promise<void> = async () => {};
  let commands: Array<{ slash: { name: string; aliases?: string[] }; onSelect: () => void }> = [];
  let dialog: { title: string; options?: Array<{ title: string; value: string }>; onSelect?: (option: { value: string }) => void } | undefined;
  try {
    await fs.writeFile(process.env.SYSTEM1ROUTER_CONFIG, JSON.stringify({ routing: { enabled: true } }));
    const api = {
      command: { register: (fn: () => typeof commands) => { commands = fn(); return () => {}; } },
      ui: {
        dialog: { replace: (render: () => typeof dialog) => { dialog = render(); }, clear: () => { dialog = undefined; } },
        DialogSelect: (props: typeof dialog) => props, DialogAlert: (props: typeof dialog) => props,
        toast: () => {},
      },
      lifecycle: { onDispose: (callback: () => Promise<void>) => { dispose = callback; } },
    };
    await System1RouterTui(api as never, undefined, {} as never);
    expect(commands.map((command) => command.slash.name)).toEqual(["s1router_toogle", "s1router_config", "s1router_stats"]);
    expect(commands[0]?.slash.aliases).toEqual(["s1router_toggle"]);
    commands[1]!.onSelect();
    await Bun.sleep(30);
    expect(dialog?.options?.map((option) => option.value)).toEqual(["toggle-router", "verbose", "config", "logs"]);
    dialog?.onSelect?.({ value: "toggle-router" });
    await Bun.sleep(50);
    expect(JSON.parse(await fs.readFile(process.env.SYSTEM1ROUTER_CONFIG, "utf8")).routing.enabled).toBe(false);

    const toml = path.join(root, "config.toml");
    await fs.writeFile(toml, "[routing]\nenabled = false\n\n[providers]\nselected = 'jev'\n");
    process.env.SYSTEM1ROUTER_CONFIG = toml;
    commands[0]!.onSelect();
    await Bun.sleep(50);
    const changed = await fs.readFile(toml, "utf8");
    expect(changed).toContain("enabled = true");
    expect(changed).toContain("selected = 'jev'");

    const directory = consentDirectory();
    const request = { id: "sample", createdAt: Date.now(), expiresAt: Date.now() + 5000,
      current: { provider: "p", model: "old" }, recommendation: { provider: "p", model: "new", reason: "Better for this task" } };
    await fs.writeFile(path.join(directory, "sample.request.json"), JSON.stringify(request));
    for (let i = 0; i < 20 && dialog?.title !== "System1Router · choose a model for this request"; i++) await Bun.sleep(50);
    expect(dialog?.options?.map((option) => option.title)).toEqual(["Use p/new", "Keep p/old"]);
    dialog?.onSelect?.({ value: "accept" });
    await Bun.sleep(30);
    const answer = JSON.parse(await fs.readFile(path.join(directory, "sample.response.json"), "utf8"));
    expect(answer.accepted).toBe(true);
  } finally {
    await dispose();
    if (oldState === undefined) delete process.env.XDG_STATE_HOME;
    else process.env.XDG_STATE_HOME = oldState;
    if (oldConfig === undefined) delete process.env.SYSTEM1ROUTER_CONFIG;
    else process.env.SYSTEM1ROUTER_CONFIG = oldConfig;
    await fs.rm(root, { recursive: true, force: true });
  }
});
