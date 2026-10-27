# System1Router

[![crates.io](https://img.shields.io/crates/v/system1router.svg)](https://crates.io/crates/system1router)
[![Release](https://github.com/Valkea/System1Router/actions/workflows/release.yml/badge.svg)](https://github.com/Valkea/System1Router/actions/workflows/release.yml)
[![Model on Hugging Face](https://img.shields.io/badge/%F0%9F%A4%97%20model-System1Router--effort-yellow)](https://huggingface.co/Valkea/System1Router-effort)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Rust 1.89+](https://img.shields.io/badge/rust-1.89%2B-orange.svg)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20(Apple%20Silicon)-lightgrey.svg)
![Works with Claude Code and OpenCode](https://img.shields.io/badge/works%20with-Claude%20Code%20%7C%20OpenCode-8A2BE2.svg)

**Sets the reasoning effort of every prompt you send to Claude Code or OpenCode, automatically.**

Coding assistants run every turn at one effort level. Set it high and "rename this variable" or "yes, go ahead" burn minutes of thinking and tokens; set it low and the hard bug gets a shallow answer. Switching by hand with `/effort` (or an OpenCode variant) every prompt is tedious, so nobody does it.

System1Router reads each prompt with the recent conversation (files in play, tools used, the last exchanges), so a short "ok, do it" after a long debugging session is still seen as hard. It predicts the effort the turn needs, `low` to `max`, and applies it before the prompt is sent:

```
you:  fix the typo in the README title          → S1R/Ettin → low
you:  the cache test is flaky under load, why?  → S1R/Ettin → xhigh
you:  ok, do it                                 → S1R/Ettin → high   (the fix it refers to)
```

- **Keeps your model and its prompt cache.** Only the effort changes, so the cached conversation is reused; switching models would throw it away.
- **Local and private.** The classifier is a small model (Ettin, 32M parameters, 130 MB) running on your machine: about 10 ms per prompt, no API key, nothing sent anywhere. One background process serves every open session and stops when the last one closes.
- **You stay in control.** Accept every suggestion, confirm it, or pick from a menu; open a prompt with `[high]` to force its effort (Claude Code); turn it off with `/s1router_toggle`.
- **Fail open.** If anything goes wrong, the prompt goes through unchanged.

The classifier was trained on 2,500 real coding prompts labelled in hindsight with the effort they needed. On held-out sessions it picks the exact level 60% of the time and is within one level 94% of the time (always guessing `medium`: 33%). See [experiments/TRAINING.md](experiments/TRAINING.md) to retrain it on your own sessions. It can also use the hosted [Jev](#jev) classifier instead.

Under the hood, the `system1router` executable reads one JSON event on stdin and writes one JSON decision on stdout; the Claude Code mod and the OpenCode plugin call it.

## Install

### 1. Get the binary: pick one

**Prebuilt binary** (no Rust needed):

```sh
curl -fsSL https://raw.githubusercontent.com/Valkea/System1Router/main/install.sh | sh
```

**Cargo** (builds from crates.io):

```sh
cargo install system1router
```

| | Needs | Installs to | Update |
|---|---|---|---|
| **Prebuilt** | Apple Silicon macOS, or Linux x86_64/arm64 with glibc 2.38+ (Ubuntu 24.04, Debian 13, Fedora 39) | `~/.local/bin` | run the `curl` line again |
| **Cargo** | Rust 1.89+, on the same systems | `~/.cargo/bin` | `system1router update` |

ONNX Runtime, which runs the local model, has no Intel Mac or older-glibc build, so neither option covers those. `install.sh` downloads the binary for your system from the [latest release](https://github.com/Valkea/System1Router/releases/latest) and checks its SHA-256 before installing it; `SYSTEM1ROUTER_INSTALL_DIR` changes the folder.

### 2. Set it up

```sh
system1router install      # Claude Code mod, OpenCode plugin, and the local model (130 MB)
system1router toggle on
```

Then restart Claude Code and OpenCode. The status line shows `S1R on · Ettin`, and each prompt shows the effort it was given.

- `system1router install` asks which assistants and which scope (all projects or this one), then downloads the model into `~/.local/share/system1router/models/`. In Claude Code it installs the mod into `~/.local/share/system1router/claude-code-mod` and names it in `env.CLAUDE_CODE_PLUGIN_DIRS` of `~/.claude/settings.json`.
- `system1router toggle on` writes `~/.config/system1router/config.json` if it does not exist. Defaults: effort routing, local model, ask before changing the effort. `system1router consent auto|confirm|menu` changes how a new effort is confirmed; `system1router doctor` checks the setup.

### From source

```sh
git clone https://github.com/Valkea/System1Router.git
cd System1Router
cargo install --path . --locked
```

For a checkout without installing, use `cargo run --` in place of `system1router`.

The earlier model-switching router lives on the `model-routing` branch.

## Configure

The config lives in `~/.config/system1router/config.json`; `toggle`, `provider` and `consent` create and edit it. JSON, or TOML when the file name ends in `.toml`. All fields: [docs/CONFIGURATION.md](docs/CONFIGURATION.md). To start from the full example instead:

```sh
mkdir -p ~/.config/system1router
cp config.example.json ~/.config/system1router/config.json
```

| Command | What |
|---|---|
| `system1router toggle [on\|off]` | Routing on or off (`/s1router_toggle` in both assistants) |
| `system1router consent auto\|confirm\|menu` | Apply each effort, confirm it (yes/no), or pick it from a menu (`/s1router_consent` in Claude Code) |
| `system1router provider local\|jev` | Which classifier decides (Laya: set `providers.selected` in the config) |
| `system1router status`, `stats`, `doctor` | State, effort counts and acceptance, setup problems |

`routing.swap` chooses what a decision changes: `effort` (default) keeps the model and sets its reasoning effort from `effort.levels`; `model` switches to the model a rule names, which needs rules and loses the prompt cache (see [Model swap](#model-swap)). Installing never changes anything by itself: nothing is classified until `system1router toggle on`.

### Classifiers

The classifier reads the prompt and its recent context and decides how hard the turn is. Pick one with `system1router provider local|jev`, or `providers.selected` in the config.

| | Runs | Predicts | Exact effort on held-out sessions | Cost |
|---|---|---|---|---|
| **Local (Ettin)** | inside `system1router`, on your machine | the effort directly | 60% | free, no key |
| **Laya** | a worker you provide (command or local HTTP) | one of six classes | depends on your worker | yours |
| **Jev** | TypeSafe's hosted service | one of six classes | 54% | API key |

- **Local** is the default because it needs nothing: no key, no network after the 130 MB download, no Python. It is trained for this exact task, on prompts labelled with the effort they needed, and you can retrain it on your own sessions. [Ettin-32M](https://huggingface.co/jhu-clsp/ettin-encoder-32m) was picked among small encoders because it matched mmBERT-small's accuracy at a quarter of the size (129 MB against 565 MB) and a third of the latency, under an MIT licence.
- **Laya** is a slot for your own classifier: any program that reads the router's request and answers one of the six classes (`question_simple` … `code_hard`), which `effort.levels` turns into an effort. Use it to plug in a model in another runtime. The local model is not served this way: a class would lose its direct effort prediction, and each user would need a separate Python runtime.
- **Jev** is TypeSafe's general System One classifier, hosted. It is not trained on effort, so its class goes through `effort.levels`; it adds a second opinion (averaging it with the local model reached 64% in [experiments/RESULTS.md](experiments/RESULTS.md)) at the cost of a key and sending the redacted prompt to TypeSafe.

<details open>
<summary><b>Local model (Ettin)</b>: default, on this machine, no key</summary>

[Ettin-32M](https://huggingface.co/jhu-clsp/ettin-encoder-32m) fine-tuned to predict the effort directly from the prompt and its context ([model card](https://huggingface.co/Valkea/System1Router-effort), [training](experiments/TRAINING.md)). It needs no key and sends nothing over the network. On held-out sessions it picks the judge's effort 60% of the time, against 54% for Jev ([experiments/RESULTS.md](experiments/RESULTS.md), section 4).

```sh
system1router provider local    # downloads the model (130 MB) the first time
```

One background process (`system1router model serve`) holds the model for every Claude Code and OpenCode session. The first prompt, or the start of a session, launches it in about half a second; afterwards a prompt takes about 10 ms. It exits 30 seconds after the last assistant that used it closes, or after `idleMinutes` without a prompt. `system1router model status|start|stop` inspects and controls it; its errors go to `~/.local/state/system1router/model.log`.

```json
"providers": {
  "selected": "local",
  "local": { "model": "ettin-32m-effort-v1", "threads": 2, "idleMinutes": 30 }
}
```

The three files (`<model>.json`, `<model>.onnx`, `<model>.tokenizer.json`) download from [Valkea/System1Router-effort](https://huggingface.co/Valkea/System1Router-effort) into `~/.local/share/system1router/models/<model>/`. `url` (a folder URL or path) or `system1router provider local --from <folder or URL>` points elsewhere, for a model you trained yourself.

</details>

<details>
<summary><b>Laya</b>: your own local worker</summary>

Laya runs as a local worker process or a local HTTP runtime that you provide. It never falls back to another provider:

```json
{
  "providers": {
    "selected": "laya",
    "laya": { "command": "laya-router-worker", "startupTimeoutMs": 1000 }
  }
}
```

For a local HTTP worker, use `endpoint`, `baseUrl`, or `LAYA_BASE_URL`:

```sh
export LAYA_BASE_URL='http://127.0.0.1:8788/classify'
```

The worker receives the same bounded request as Jev and returns the normalized classification directly or under `classification`. Use `command` for a managed local process, or `baseUrl` for a separately supervised runtime. A missing or unhealthy worker means no classification, and the prompt goes through unchanged.

</details>

<details>
<summary><b>Jev</b>: hosted by TypeSafe, needs an API key</summary>

Keep the key out of the JSON file: put it in an environment variable, or in `~/.config/system1router/.env` for hooks and GUI-launched assistants (exported variables win). Keep that file private and never commit it.

```dotenv
JEV_API_KEY=
JEV_BASE_URL=https://api.typesafe.ai/v1/systemone
```

```json
{
  "providers": {
    "selected": "jev",
    "jev": { "baseUrl": "https://api.typesafe.ai/v1/systemone", "credentialEnv": "JEV_API_KEY", "model": "jev-latest" }
  }
}
```

Use `endpoint` when TypeSafe gives you an exact System One endpoint, or `baseUrl`/`JEV_BASE_URL` otherwise. The router sends a bounded, redacted request with routing questions and maps the answers to a class, which `effort.levels` turns into an effort. Calls time out after 3 seconds by default. `system1router doctor` reports whether the endpoint and key are set, never the key.

</details>

## Assistants

`system1router install` lists the detected assistants and asks which to install for, then **This project only** or **All projects for this user**. For scripts: `--host opencode|claude-code|all` and `--scope project|global`. `system1router uninstall --host <host> --scope <scope>` removes only what the installer created. Restart the assistant afterwards.

- **Claude Code**: the System1Router mod, written to `~/.local/share/system1router/claude-code-mod` and named in `env.CLAUDE_CODE_PLUGIN_DIRS` of the settings. Before each prompt is sent it sets the turn's effort, after a native dialog if `consent` asks for one, and shows the decision in the status line (`S1R/Ettin → xhigh`; with Jev, its class too: `S1R/Jev · code_hard → xhigh`). `/s1router` opens a pane with the state and recent decisions; `/s1router_consent` chooses how an effort is confirmed. Open a prompt with `[low]` … `[max]` to choose its effort yourself; pressing Esc and resending with a tag records a correction. A `UserPromptSubmit` hook adds `/s1router_toggle`, `/s1router_config` and `/s1router_stats`, which run without a model turn. Details: [docs/CLIENTS.md](docs/CLIENTS.md#claude-code-mod).
- **OpenCode**: server and TUI plugins under `.opencode/` or `~/.config/opencode/`. The effort becomes the model's nearest variant for that request. `/s1router_config` opens a menu to toggle routing, switch the classifier, show traces and copy the config path; `/s1router_stats` shows counts and acceptance. Each prompt starts with a compact `S1R/Ettin` block (Jev adds its class), excluded from model context.

What the classifier sees: the current prompt plus recent conversation, tool use and file references, redacted and bounded by `redaction.maxContextChars` (4,000 characters). Provider exchanges are logged locally in `~/.local/state/system1router/provider-debug.jsonl`, usage stats in `usage.jsonl`.

<details>
<summary><b>Model swap</b> (<code>routing.swap: "model"</code>)</summary>

<a id="model-swap"></a>
Rules map a class to a target model; the assistant switches to it for that request. Switching models discards the prompt cache, which is why effort is the default. The earlier version built around this lives on the `model-routing` branch.

In OpenCode a native dialog offers the suggested model before generation (**Use suggested** by default; **Always switch** / **Never switch for this pair** remember the answer; Escape or the `timeouts.consentMs` countdown keeps the current model). In Claude Code the mod switches the model of the turn; the plain hook can only show the suggestion and its `/model` command. Rules for Claude Code use `"host": "claude-code"` and Anthropic targets (`opus`, `sonnet`, `haiku`). `system1router prices --refresh` shows each target's price (LiteLLM table), used by the cache guard. See [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

</details>

<details>
<summary><b>Other hosts</b> (Codex, Copilot, generic): advisory command hooks</summary>

Install one ownership-marked hook per host; repeating `init` is idempotent and `uninstall` removes only System1Router-owned entries. `--host-config <path>` targets a non-default host config.

```sh
system1router init --host codex
system1router init --host copilot-cli
system1router init --host copilot-vscode
system1router init --host generic
```

Smoke-test the protocol without a real client:

```sh
printf '%s\n' '{"protocolVersion":"1","host":"generic","event":"user_prompt","sessionId":"demo","request":{"query":"Explain this error"}}' | system1router
```

</details>

## Commands

```sh
system1router --version
system1router --help
system1router install --scope global
system1router toggle on
system1router consent menu
system1router provider local
system1router model status
system1router update --check
system1router doctor --config ~/.config/system1router/config.json
system1router prices --refresh
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

Provider input is bounded and redacted. Local telemetry stores correlation IDs, classification metadata, policy outcome, mode, latency, and error category. Prompts, credentials, attachments, tool arguments, and model output are excluded by default; `telemetry.capturePrompts` opts in to recording the redacted request and context the provider saw, locally, for evaluating a trial. Set `SYSTEM1ROUTER_TELEMETRY_DISABLED=1` to disable local telemetry and enable debug capture only through an explicit, redacted deployment-specific mechanism.
