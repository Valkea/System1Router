import { test, expect } from "bun:test";
import { promises as fs } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { System1RouterTui } from "./tui";
import { consentDirectory, consentTimeoutMs } from "./shared";

test("loads router commands and handles confirmation, cancel and timeout", async () => {
  const root = await fs.mkdtemp(path.join(tmpdir(), "system1router-tui-"));
  const oldState = process.env.XDG_STATE_HOME;
  const oldConfig = process.env.SYSTEM1ROUTER_CONFIG;
  process.env.XDG_STATE_HOME = path.join(root, "state");
  process.env.SYSTEM1ROUTER_CONFIG = path.join(root, "config.json");
  let dispose: () => Promise<void> = async () => {};
  let commands: Array<{ slash: { name: string; aliases?: string[] }; onSelect: () => void }> = [];
  type Dialog = { title: string; message?: string; options?: Array<{ title: string; value: string }>;
    onSelect?: (option: { value: string }) => void; onConfirm?: () => void; onCancel?: () => void };
  let dialog: Dialog | undefined;
  let closeDialog: (() => void) | undefined;
  try {
    await fs.writeFile(process.env.SYSTEM1ROUTER_CONFIG, JSON.stringify({ routing: { enabled: true } }));
    expect(await consentTimeoutMs()).toBe(15000);
    const api = {
      command: { register: (fn: () => typeof commands) => { commands = fn(); return () => {}; } },
      ui: {
        dialog: {
          replace: (render: () => Dialog, onClose?: () => void) => {
            const previous = closeDialog;
            closeDialog = onClose;
            dialog = render();
            previous?.();
          },
          clear: () => {
            const previous = closeDialog;
            closeDialog = undefined;
            dialog = undefined;
            previous?.();
          },
        },
        DialogSelect: (props: Dialog) => props,
        DialogConfirm: (props: Dialog) => props,
        DialogAlert: (props: Dialog) => props,
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
    await fs.writeFile(toml, "[routing]\nenabled = false\n\n[providers]\nselected = 'jev'\n\n[timeouts]\nconsentMs = 90_000\n");
    process.env.SYSTEM1ROUTER_CONFIG = toml;
    commands[0]!.onSelect();
    await Bun.sleep(50);
    const changed = await fs.readFile(toml, "utf8");
    expect(changed).toContain("enabled = true");
    expect(changed).toContain("selected = 'jev'");
    expect(await consentTimeoutMs()).toBe(90000);

    const directory = consentDirectory();
    const request = { id: "sample", createdAt: Date.now(), expiresAt: Date.now() + 5000,
      current: { provider: "p", model: "old" }, recommendation: { provider: "p", model: "new", reason: "Better for this task" } };
    await fs.writeFile(path.join(directory, "sample.request.json"), JSON.stringify(request));
    for (let i = 0; i < 20 && dialog?.title !== "Use p/new?"; i++) await Bun.sleep(50);
    expect(dialog?.title).toBe("Use p/new?");
    expect(dialog?.message).toContain("Current: p/old");
    expect(dialog?.message).toContain("timeout keeps the current model");
    expect(dialog?.options).toBeUndefined(); // DialogConfirm has no search field.
    dialog?.onConfirm?.(); // Confirm is highlighted by default in OpenCode.
    await Bun.sleep(30);
    expect(JSON.parse(await fs.readFile(path.join(directory, "sample.response.json"), "utf8")).accepted).toBe(true);

    const cancelled = { ...request, id: "cancelled", createdAt: Date.now(), expiresAt: Date.now() + 3000 };
    await fs.writeFile(path.join(directory, "cancelled.request.json"), JSON.stringify(cancelled));
    for (let i = 0; i < 20 && dialog?.title !== "Use p/new?"; i++) await Bun.sleep(50);
    api.ui.dialog.clear();
    for (let i = 0; i < 20; i++) {
      if (await fs.access(path.join(directory, "cancelled.response.json")).then(() => true).catch(() => false)) break;
      await Bun.sleep(25);
    }
    expect(JSON.parse(await fs.readFile(path.join(directory, "cancelled.response.json"), "utf8")).accepted).toBe(false);

    const expiring = { ...request, id: "expiring", createdAt: Date.now(), expiresAt: Date.now() + 1100 };
    await fs.writeFile(path.join(directory, "expiring.request.json"), JSON.stringify(expiring));
    for (let i = 0; i < 60; i++) {
      if (await fs.access(path.join(directory, "expiring.response.json")).then(() => true).catch(() => false)) break;
      await Bun.sleep(50);
    }
    expect(JSON.parse(await fs.readFile(path.join(directory, "expiring.response.json"), "utf8")).accepted).toBe(false);
  } finally {
    await dispose();
    if (oldState === undefined) delete process.env.XDG_STATE_HOME;
    else process.env.XDG_STATE_HOME = oldState;
    if (oldConfig === undefined) delete process.env.SYSTEM1ROUTER_CONFIG;
    else process.env.SYSTEM1ROUTER_CONFIG = oldConfig;
    await fs.rm(root, { recursive: true, force: true });
  }
});
