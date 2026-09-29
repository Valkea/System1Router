# Client Adapters

Adapters are installed in this order: OpenCode, Claude Code, Codex, GitHub Copilot CLI/VS Code, then the generic hook contract.

| Host | Installation | Default capability | Model mutation |
| --- | --- | --- | --- |
| OpenCode | `system1router install` (project or global scope) | consent-gated full override | Model choice picker runs before generation; acceptance changes this user message's model and variant |
| Claude Code | `system1router install --host claude-code` (project or global scope) | advisory | Hooks cannot change the request's model; the suggestion and the `/model` command are shown to the user |
| Codex | `system1router init --host codex` | consent-only | tool hooks are not model-request hooks |
| Copilot CLI | `system1router init --host copilot-cli` | observe-only | documented prompt fields only |
| Copilot VS Code | `system1router init --host copilot-vscode` | observe-only | documented prompt/tool fields only |
| Generic | `system1router init --host generic` | observe-only | normalized stdin/stdout contract only |

`system1router install` detects OpenCode and Claude Code (on `PATH` or by their config directory) and installs for the ones you pick. Installers add one System1Router entry and preserve unrelated configuration and key order. `uninstall` removes only entries carrying the System1Router ownership marker. Run `doctor` after host upgrades. Gateway mode is available only to clients that can explicitly use the configured compatible endpoint; it is never enabled by installing a hook.

For another assistant, add its host normalization and decision translation in `src/adapters.rs`, its hook installer in `src/host_install.rs`, and report its actual capabilities via `Host::capability`. The classifier, policy resolver, protocol and telemetry stay host-independent. OpenCode's server and TUI adapters share their config, consent and usage contract in `.opencode/plugins/system1router/shared.ts`.

`system1router install` installs the server and TUI plugins for the current project or globally for the user; the scope menu is interactive, or select `--scope project|global`. The server hook pauses the incoming message before it is saved or sent to the model. A native model-choice picker shows the routing category, current and suggested models, reason, and live countdown. Moving between **Use suggested** and **Keep current** resets the configured timer. Escape, timeout, or a missing TUI keeps the current model. Set `timeouts.consentMs` in the router config to change the decision time (15 seconds by default). The project must be opened in the interactive OpenCode TUI for this picker; headless runs fail open without switching.

Use `/s1router_toogle` (or `/s1router_toggle`) to toggle routing, `/s1router_config` to open routing and verbose controls and copy the config or log path, and `/s1router_stats` for per-model call counts and acceptance rate. A status notification appears when routing starts, followed by a compact S1R/class/model/variant block before the prompt. Classification receives the unchanged current prompt plus recent conversation text, tool results/usage, and file references, bounded and redacted by `redaction.maxContextChars` (4,000 characters by default). Verbose mode adds that router input and the redacted Jev request/response in the native Thought section; the status and trace parts are ignored by model context. The consent picker displays the routing category and a live countdown, which resets when moving between suggested/current model choices. Declining a suggested model preserves the current model and omits the status block. Full provider exchanges are written to `~/.local/state/system1router/provider-debug.jsonl` (or `$XDG_STATE_HOME/system1router/provider-debug.jsonl`). OpenCode records the *actual* accepted model after the consent dialog, while the native core records metadata for other hosts. The native executable reads config and `.env` on each request; plugin files and TUI config need a restart.

## Claude Code

The installer adds a `UserPromptSubmit` command hook to `~/.claude/settings.json` (global) or `<project>/.claude/settings.local.json` (project, not committed), using the absolute path of the executable:

```json
{ "hooks": { "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": "/home/me/.cargo/bin/system1router hook --host claude-code", "timeout": 10 } ] } ] } }
```

For each prompt the hook reads the tail of the session transcript for context (recent turns, tools, files), the current model and the cached context size, then classifies the prompt. When a rule suggests a different model it prints a notice such as `S1R · code_hard (80%) → opus instead of claude-sonnet-5-5. Run /model opus to switch; …`. The installer also adds `/s1router_toggle` (optionally `on`/`off`), `/s1router_config` (status, config and log paths) and `/s1router_stats` to `commands/` next to the settings file. The hook answers these itself and blocks the prompt, so they cost no model turn; the answer appears as a hook notice. The command files contain only a fallback instruction (no `!` execution, which Claude Code would run before the hook).

Nothing is added to the model's context, other slash commands are ignored, and any router failure exits 0 so Claude Code never reports a hook error. Rule targets can use aliases (`opus`, `sonnet`, `haiku`) or full model IDs. Verified with Claude Code 2.1.285.

Codex and Copilot installers are unchanged and unverified against current host versions.

## Generic hook contract

```sh
printf '%s\n' '{"protocolVersion":"1","host":"generic","event":"user_prompt","sessionId":"s1","request":{"query":"..."}}' | system1router
```

The request and response are single JSON objects. Stdout must contain no logs. Unsupported response fields are removed by each translator, and advisory adapters never resubmit or rewrite a request.
