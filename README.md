# System1Router

System1Router is a fail-open, context-aware model-routing core for AI coding clients. The native `system1router` executable accepts one normalized JSON event on stdin and emits one JSON decision on stdout. Diagnostics are written to stderr only.

## Build

### From source

The repository currently ships source packages rather than prebuilt release archives. Install Rust 1.78 or newer, then install the executable into Cargo's binary directory:

```sh
cargo install --path . --locked
system1router --version
```

On Windows PowerShell, use the same `cargo install` command and ensure `%USERPROFILE%\.cargo\bin` is on `PATH`. For a local checkout without installing, use `cargo run --` in place of `system1router` below.

The binary supports Linux, macOS, and Windows on the Rust-supported x86_64 and aarch64 targets. Release packaging uses `cargo build --release --target <target>` and publishes the resulting `system1router` executable with the matching host adapter files.

## Configure

Create a configuration file and point the executable at it:

```sh
mkdir -p "$HOME/.config/system1router"
cp config.example.json "$HOME/.config/system1router/config.json"
export SYSTEM1ROUTER_CONFIG="$HOME/.config/system1router/config.json"
```

Routing is disabled by default. Set `routing.enabled` to `true` only after a provider and at least one catalog-validated rule are configured. Configuration accepts JSON, or TOML when the file name ends in `.toml`. See `docs/CONFIGURATION.md` for all fields.

Installing the plugin is intentionally not the same as enabling routing: installation must not change a host’s model or send prompts to a provider before the user configures consent, privacy, provider credentials, and policy. On the first OpenCode message, the plugin displays a one-time status notification explaining that routing is disabled. Run `system1router doctor` to inspect the state, then enable routing after provider setup.

### Jev

Keep the Jev API key out of the JSON file. Put it in an environment variable and reference the variable by name:

```sh
export JEV_API_KEY='replace-with-your-key'
export JEV_BASE_URL='https://api.typesafe.ai/v1/systemone'
```

As a fallback for hooks and GUI-launched clients, the executable also loads `~/.config/system1router/.env`:

```dotenv
# Replace the empty value with your Jev key.
JEV_API_KEY=
JEV_BASE_URL=https://api.typesafe.ai/v1/systemone
```

Already-exported process variables take precedence over `.env`. `system1router doctor` reports Jev as unconfigured until the key is non-empty. Keep this file private and never commit it.

```json
{
  "providers": {
    "selected": "jev",
    "jev": {
      "baseUrl": "https://api.typesafe.ai/v1/systemone",
      "credentialEnv": "JEV_API_KEY",
      "model": "jev-latest"
    }
  }
}
```

Use `endpoint` when TypeSafe gives you an exact System One endpoint, or `baseUrl`/`JEV_BASE_URL` for deployment-provided URL configuration. The adapter sends a bounded TypeSafe System One request with `state`, `model`, and routing questions, then maps the `answers` response into the normalized routing classification. Native HTTP and HTTPS transport are supported. Remote provider calls allow up to 3 seconds by default; hook execution allows up to 6 seconds. `doctor` only reports whether the endpoint and credential are configured, never the key.

### Laya

Laya can run as a local worker process or as a configured local HTTP runtime. It never falls back to Jev implicitly:

```json
{
  "providers": {
    "selected": "laya",
    "laya": {
      "command": "laya-router-worker",
      "startupTimeoutMs": 1000
    }
  }
}
```

For a local HTTP worker, use `endpoint`, `baseUrl`, or `LAYA_BASE_URL`:

```sh
export LAYA_BASE_URL='http://127.0.0.1:8788/classify'
```

The worker receives the same bounded decision request as Jev and returns the normalized classification directly or under `classification`. Use `command` for a managed local process, or `baseUrl` for a separately supervised local runtime. A missing or unhealthy Laya runtime produces a fail-open classification-unavailable outcome.

## Install Host Hooks

Inspect the detected host contracts and current provider/configuration state first:

```sh
system1router doctor
```

Install one ownership-marked hook per host. Repeating `init` is idempotent and `uninstall` removes only System1Router-owned entries:

```sh
system1router init --host claude-code
system1router init --host codex
system1router init --host copilot-cli
system1router init --host copilot-vscode
system1router init --host generic
```

Use `--host-config /path/to/host-config.json` for a test or non-default host configuration. OpenCode loads the server and TUI entries from `.opencode/opencode.json` and `.opencode/tui.json`; ensure `system1router` is on `PATH` and restart OpenCode after plugin changes. In the interactive TUI, System1Router displays a native Yes/Cancel confirmation before generation. Accepting applies the recommended model and variant to that same request; declining or timing out keeps the current model. Headless OpenCode runs fail open without changing models.

OpenCode offers three router commands: `/s1router_toogle` (also accepts `/s1router_toggle`) flips routing on/off; `/s1router_config` opens the menu to toggle routing and verbose traces or copy the config and provider-log paths; `/s1router_stats` shows calls by model and the share of answered suggestions accepted. The config file must exist to use the toggle. Routing status appears before the model starts thinking; the final status is shown with the prompt, excluded from model context. Verbose mode shows the redacted System One request and response in the chat, also excluded from model context. Provider exchanges are stored in `~/.local/state/system1router/provider-debug.jsonl` (or `$XDG_STATE_HOME/system1router/provider-debug.jsonl`). Usage stats are stored locally in `usage.jsonl` alongside the core's `events.jsonl`; disabling telemetry disables new stats. The native executable reads config and `.env` for every request; changes to plugin files need an OpenCode restart.

The model confirmation has no search field and highlights **Confirm** (use the suggested model) by default. Use left/right arrows to choose **Cancel** (keep the current model), Enter to select, or Escape to keep the current model. It shows the time available when opened; set `timeouts.consentMs` in the active router config to extend that time (e.g. `120000` for two minutes). Expiry keeps the current model.

Smoke-test the normalized protocol without intercepting a real client:

```sh
printf '%s\n' '{"protocolVersion":"1","host":"generic","event":"user_prompt","sessionId":"demo","request":{"query":"Explain this error"}}' | system1router
```

## Commands

```sh
system1router --version
system1router doctor --config ~/.config/system1router/config.json
system1router init --host claude-code
system1router uninstall --host claude-code
system1router hook --host codex < event.json
system1router gateway --config ~/.config/system1router/config.json
```

The executable always fails open unless `routing.failOpen` is set to `false`. See `docs/CONFIGURATION.md`, `docs/CLIENTS.md`, and `docs/OPERATIONS.md` for configuration, capability, privacy, and recovery details.

## Protocol

The normalized protocol is version `1`. A request contains `protocolVersion`, `host`, `event`, `sessionId`, `request`, bounded context, current model state, model catalog, capabilities, and an optional `correlationId`. A response has one of `allow`, `block`, `ask`, `bypass`, or `route` outcomes. Model recommendations are advisory metadata unless the host reports `full-override` or the request uses an explicitly configured gateway.

The protocol exit codes are `10` malformed event, `11` unsupported version, `12` startup failure, `13` timeout, and `14` internal error. A normal fail-open decision is still emitted as JSON where an event is available.

## Privacy

Provider input is bounded and redacted. Local telemetry stores correlation IDs, classification metadata, policy outcome, mode, latency, and error category. Prompts, credentials, attachments, tool arguments, and model output are excluded by default. Set `SYSTEM1ROUTER_TELEMETRY_DISABLED=1` to disable local telemetry and enable debug capture only through an explicit, redacted deployment-specific mechanism.
