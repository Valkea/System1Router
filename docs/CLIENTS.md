# Client Adapters

Adapters are installed in this order: OpenCode, Claude Code, Codex, GitHub Copilot CLI/VS Code, then the generic hook contract.

| Host | Installation | Default capability | Model mutation |
| --- | --- | --- | --- |
| OpenCode | `system1router install` (project or global scope) | consent-gated full override | Model choice picker runs before generation; acceptance changes this user message's model and variant |
| Claude Code | `system1router init --host claude-code` | consent-only | `PreModelSwitch` can report decisions but does not initiate a switch |
| Codex | `system1router init --host codex` | consent-only | tool hooks are not model-request hooks |
| Copilot CLI | `system1router init --host copilot-cli` | observe-only | documented prompt fields only |
| Copilot VS Code | `system1router init --host copilot-vscode` | observe-only | documented prompt/tool fields only |
| Generic | `system1router init --host generic` | observe-only | normalized stdin/stdout contract only |

Installers add one ownership-marked entry and preserve unrelated configuration. `uninstall` removes only entries carrying the System1Router ownership marker. Run `doctor` after host upgrades. Gateway mode is available only to clients that can explicitly use the configured compatible endpoint; it is never enabled by installing a hook.

For another assistant, add its host normalization and decision translation in `src/adapters.rs`, its hook installer in `src/host_install.rs`, and report its actual capabilities via `Host::capability`. The classifier, policy resolver, protocol and telemetry stay host-independent. OpenCode's server and TUI adapters share their config, consent and usage contract in `.opencode/plugins/system1router/shared.ts`.

`system1router install` installs the server and TUI plugins for the current project or globally for the user; the scope menu is interactive, or select `--scope project|global`. The server hook pauses the incoming message before it is saved or sent to the model. A native model-choice picker shows the routing category, current and suggested models, reason, and live countdown. Moving between **Use suggested** and **Keep current** resets the configured timer. Escape, timeout, or a missing TUI keeps the current model. Set `timeouts.consentMs` in the router config to change the decision time (15 seconds by default). The project must be opened in the interactive OpenCode TUI for this picker; headless runs fail open without switching.

Use `/s1router_toogle` (or `/s1router_toggle`) to toggle routing, `/s1router_config` to open routing and verbose controls and copy the config or log path, and `/s1router_stats` for per-model call counts and acceptance rate. A status notification appears when routing starts, followed by a compact S1R/class/model/variant block before the prompt. Classification receives the unchanged current prompt plus recent conversation text, tool results/usage, and file references, bounded and redacted by `redaction.maxContextChars` (4,000 characters by default). Verbose mode adds that router input and the redacted Jev request/response in the native Thought section; the status and trace parts are ignored by model context. The consent picker displays the routing category and a live countdown, which resets when moving between suggested/current model choices. Declining a suggested model preserves the current model and omits the status block. Full provider exchanges are written to `~/.local/state/system1router/provider-debug.jsonl` (or `$XDG_STATE_HOME/system1router/provider-debug.jsonl`). OpenCode records the *actual* accepted model after the consent dialog, while the native core records metadata for other hosts. The native executable reads config and `.env` on each request; plugin files and TUI config need a restart.

## Generic hook contract

```sh
printf '%s\n' '{"protocolVersion":"1","host":"generic","event":"user_prompt","sessionId":"s1","request":{"query":"..."}}' | system1router
```

The request and response are single JSON objects. Stdout must contain no logs. Unsupported response fields are removed by each translator, and advisory adapters never resubmit or rewrite a request.
