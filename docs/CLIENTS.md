# Client Adapters

Adapters are installed in this order: OpenCode, Claude Code, Codex, GitHub Copilot CLI/VS Code, then the generic hook contract.

| Host | Installation | Default capability | Model mutation |
| --- | --- | --- | --- |
| OpenCode | `system1router install` (project or global scope) | consent-gated full override | Model choice picker runs before generation; acceptance changes this user message's model and variant |
| Claude Code | `system1router install --host claude-code` (project or global scope) | advisory | Hooks cannot change the request's model; the suggestion and the `/model` command are shown to the user |
| Claude Code mod | `claude --plugin-dir claude-code/system1router` (see below) | consent-gated per-turn override | The mod sends every model request of the turn to the accepted model and effort; the session's model is unchanged |
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

For each prompt the hook reads the tail of the session transcript for context (recent turns, tools, files), the current model and the cached context size, then classifies the prompt. Before the first reply of a session the current model comes from `ANTHROPIC_MODEL` or the `model` settings key (a `--model` flag is not visible to hooks).

A hook cannot change the model of the prompt being submitted, so with `consent.mode: "interactive"` (the default) the choice is made **before** anything is sent: the prompt is held and Claude Code shows

```
S1R · code_hard (80%) suggests opus instead of claude-sonnet-5-5. Complex change.
  → switch: /model opus then send .
  → keep claude-sonnet-5-5: send .
Your message is held and will be sent as is; nothing was sent yet.
```

Sending `.` (or the same prompt again) releases the held message, which Claude receives as context, without a second classification. The choice is recorded for `/s1router_stats` by comparing the model in use with the suggestion. Held prompts expire after 30 minutes; any other prompt replaces them. Note that Claude Code's `/model` also saves the model as your default for new sessions. With any other consent mode the hook only shows the suggestion as a notice and never holds prompts. The installer also adds `/s1router_toggle` (optionally `on`/`off`), `/s1router_config` (status, config and log paths) and `/s1router_stats` to `commands/` next to the settings file. The hook answers these itself and blocks the prompt, so they cost no model turn; the answer appears as a hook notice. The command files contain only a fallback instruction (no `!` execution, which Claude Code would run before the hook).

Nothing is added to the model's context, other slash commands are ignored, and any router failure exits 0 so Claude Code never reports a hook error. Rule targets can use aliases (`opus`, `sonnet`, `haiku`) or full model IDs. Verified with Claude Code 2.1.285.

Codex and Copilot installers are unchanged and unverified against current host versions.

## Claude Code mod

Claude Code mods are plugins of function hooks that run inside Claude Code. `claude-code/system1router/` is one, and unlike the command hook it can change the model a turn runs on. The mod API is early access and may change between Claude Code releases; this mod is verified with Claude Code 2.1.294.

Load it for one session with `claude --plugin-dir /path/to/system1router/claude-code/system1router`, or for every session by adding that folder to `CLAUDE_CODE_PLUGIN_DIRS` in the `env` block of `~/.claude/settings.json`. The mod calls the `system1router` executable on `PATH`; set its `executable` option (`pluginConfigs.system1router.options.executable` in user settings) to use another path.

For each prompt the person types, the mod runs `system1router decide` with the session id, directory, prompt and the session's model. The binary finds the transcript, classifies as the hook does, and answers what to do:

- **ask**: a native dialog offers **Use opus · high**, **Keep sonnet 5.5**, **Always switch to opus** and **Never switch to opus**. Escape keeps the current model. Always/Never are stored in `consent-preferences.json`, the file OpenCode's dialog uses, so a remembered pair applies to both hosts.
- **switch** / **keep**: a remembered answer, or a non-interactive `consent.mode`, decided without asking.
- **none**: routing is off, nothing better was found, or the switch would discard a warm prompt cache.

With `routing.swap: "effort"` the suggestion keeps the model and changes its effort. The dialog offers **Use xhigh effort**, **Keep medium effort**, **Always medium → xhigh** and **Never medium → xhigh**, An accepted effort is then sent with every main-loop request of this and the following turns, until the router picks another effort. Claude Code sends a changed effort both as the request's effort and as a per-turn system message (the `per-turn-control` beta), and the prompt cache survives it: measured with Claude Code 2.1.310, a turn after an effort change read 94–100% of its prompt from the cache, against 22–24% after a model switch (`experiments/RESULTS.md`). The router therefore never holds back an effort change in Claude Code. Subagents keep their own effort. When the session's own effort changes while the router holds one (the person ran `/effort` or `/model`), the person's choice wins and automatic effort pauses for the session; the mod records it with `system1router override`, the strongest sign the router chose wrong.

To set the effort of one prompt yourself, open it with a tag: `[xhigh] refactor the cache layer` (any of `[low]`, `[medium]`, `[high]`, `[xhigh]`, `[max]`). The mod removes the tag before the prompt is sent, runs that turn at that effort, and keeps automatic effort on for the next prompts. It still asks the router what it would have chosen and records your tag as an override of it. If a turn started at the wrong effort, press Esc and send the same prompt again with a tag within two minutes: the tag is recorded as a correction of that decision, with no second classification. (A `!` prefix is not used: it is Claude Code's shell mode.) With `consent.style: "menu"`, picking another effort than the suggested one in the dialog is recorded the same way. `decide` also gets the effort the session's requests carry and the time since the last turn ended, so the router can tell a warm cache from an expired one.

An accepted model switch applies to every model request of that turn (`turn.step`), with the rule's variant as the effort when it is one of `low`, `medium`, `high`, `xhigh` or `max`. Subagents keep their own models, and the next prompt starts again from the session's model, which `/model` still controls. The API needs full model ids, so `decide` turns a rule's alias into candidate ids, newest first: Claude Code's own `ANTHROPIC_DEFAULT_<ALIAS>_MODEL` when set, otherwise the price table's ids and the current model's generation (`haiku` on `claude-opus-5-5` tries `claude-haiku-5-5`). If the API refuses an id, the mod tries the next one, then runs the turn on the session's model and says so in a toast.

The status line shows the last decision (`S1R · code_hard → opus · high`, `S1R · question_simple · kept sonnet 5.5`, `S1R · code_hard → xhigh effort`, `S1R · question_simple · xhigh effort kept for the cache (not low)`, `S1R off`). `/s1router` opens a dashboard pane: routing status with an on/off button, this session's routed prompts, stats across hosts, and remembered answers with **Forget** buttons. With the effort swap it also shows the effort per class, the effort held for the session with **Pause auto** (`a`) and **Session effort** (`e`) buttons, and the share of the prompt the cache served on turns right after an effort change against the other turns, which measures what a change really costs in this session. The mod also answers `/s1router_consent [auto|confirm|menu] [ask-below 80%|off]` (how effort suggestions are confirmed; alone it opens a dialog to choose, and `c` in the pane cycles it), `/s1router_toggle [on|off]`, `/s1router_config` and `/s1router_stats`, registering them when the hook's command files are absent. Answers are recorded with `system1router choice`, and the dashboard reads `system1router panel`; `system1router forget [KEY]` forgets one remembered answer or all of them.

The mod sets `SYSTEM1ROUTER_CLAUDE_MOD=1` for its session. The command hook, when also installed, then leaves routing to the mod and only answers the `/s1router_*` commands, so prompts are never classified twice. In `claude -p` runs nobody can be asked: only remembered answers and non-interactive consent modes switch. Run the mod's tests with `claude plugin test claude-code/system1router`.

## Generic hook contract

```sh
printf '%s\n' '{"protocolVersion":"1","host":"generic","event":"user_prompt","sessionId":"s1","request":{"query":"..."}}' | system1router
```

The request and response are single JSON objects. Stdout must contain no logs. Unsupported response fields are removed by each translator, and advisory adapters never resubmit or rewrite a request.
