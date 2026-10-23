# Configuration

Use JSON (or TOML when the file ends in `.toml`) at `SYSTEM1ROUTER_CONFIG`, `$XDG_CONFIG_HOME/system1router/config.json`, or `%APPDATA%/system1router/config.json`. Install the executable first with `cargo install --path . --locked`; the full setup flow is in the root `README.md`.

Minimal advisory configuration:

```json
{
  "routing": { "enabled": true, "failOpen": true, "minConfidence": 0.7 },
  "providers": { "selected": "laya", "laya": { "command": "laya-router-worker" } },
  "consent": { "mode": "always-decline" },
  "rules": [
    {
      "id": "hard-code",
      "priority": 10,
      "routingClass": "code_hard",
      "taskTypes": ["code-generation", "debugging"],
      "mode": "advisory",
      "target": { "provider": "openai", "model": "gpt-5", "variant": "high" },
      "reason": "Use the strongest configured model for high-risk code work"
    }
  ]
}
```

The default (`routing.enabled: false`) is a safety state, not a working routing policy. The installed OpenCode plugin remains loaded and records redacted diagnostics, but it will not classify or recommend models until a provider is selected, a policy rule is present, and routing is enabled. The plugin shows a one-time notification for this state.

Rules are evaluated by ascending `priority`, then configuration order. A rule can match routing class, task type, host, project/session scope, minimum confidence, and `branch=...`, `repository=...`, or `cwd=...` predicates. Targets are checked against the host model catalog. If a matching model is missing from the catalog, the router keeps the current model and reports why. Configure rules for the classes you want to route; an optional `defaultRecommendation` with a target catches classes without a specific rule. Targets must use the exact provider and model IDs exposed by the host. The `reason` on each rule or default recommendation controls the explanation shown to the user; if omitted, the router describes the actual matching rule, class and task type. Gateway targets must be in `gateway.upstreamAllowlist`.

Variants are configured per target with `target.variant`; they are not a separate global setting. OpenCode reports each model's enabled variants, and the router rejects a rule whose variant the model does not offer (the request keeps the current model and the reason is reported). A target without `variant` matches the current model at whatever variant is selected.

Rules without `host` apply to every host, but a host that exposes no model catalog (Claude Code) is only offered targets on its own provider, so OpenAI rules are never suggested inside Claude Code. Add `"host": "claude-code"` rules with Anthropic targets for it:

```json
{ "id": "cc-hard", "host": "claude-code", "routingClass": "code_hard", "target": { "provider": "anthropic", "model": "opus" }, "reason": "Complex or high-risk change" }
```

Run `system1router doctor` after editing: `problems` lists what prevents routing (disabled, unusable provider, no rule for an installed host) and `advice` flags price inversions, such as a simple class routed to a pricier model than a hard class.

## Model swap or effort swap

`routing.swap` chooses what a decision changes:

- `model` (default): a matching rule switches to its target model, as described above. A different model cannot reuse the conversation's prompt cache.
- `effort`: rules are not used. The current model stays and its reasoning effort is set to the level `effort.levels` gives the class (Claude Code `/effort`, an OpenCode variant):

```json
{
  "routing": { "enabled": true, "swap": "effort" },
  "consent": { "mode": "always-accept" },
  "effort": {
    "levels": { "question_simple": "low", "question_moderate": "medium", "question_hard": "high", "code_simple": "medium", "code_moderate": "high", "code_hard": "xhigh" },
    "cacheGuardTokens": 50000,
    "cacheTtlMs": 300000
  }
}
```

The levels shown are the defaults; a class left out keeps its default. Efforts are ordered `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`. A host with a catalog (OpenCode) only gets the model's own variants: the nearest one to the level, ties going to more effort; a model without variants is left alone. Claude Code silently lowers an effort the model does not take.

Whether an effort swap keeps the prompt cache depends on how the host sends it. Claude Code sends the change as a per-turn message and keeps the cache (measured in `experiments/RESULTS.md`), so it is never held back there. A host that changes the request's top-level effort (`output_config.effort`, OpenAI `reasoning.effort`) re-writes the cached conversation on the next request, as the Anthropic and OpenAI docs state. For such hosts, with `routing.cacheAware`, a lower effort waits while the context holds at least `effort.cacheGuardTokens` tokens and the last response is younger than `effort.cacheTtlMs`; a higher effort always applies. A held effort is reported as `cache_sticky`, and a class that keeps the current effort as `effort_unchanged`. Set `cacheGuardTokens` to `0` to never wait. `consent.mode` applies as for models: `interactive` asks, `always-accept` applies the effort without asking.

## Accuracy and efficiency

- **Class selection** (`routing.classSelection`, default `median`). The six classes are two ordinal scales. When the provider returns probabilities, the router picks code vs question by total probability mass and the difficulty tier by the median of that distribution, instead of the single top class. This avoids choosing *simple* when most of the mass says *moderate or harder*. Use `argmax` to keep the provider's top choice.
- **What the classifier sees.** The Jev `state` is compact labelled text (files, tools, recent conversation, then the current request), with empty fields omitted. The default criteria are the canonical class definitions, and the instruction tells the classifier that `code_*` means the assistant is expected to change files or run commands.
- **Skipped calls.** The provider is not called for empty prompts or Claude Code slash commands, or when no applicable rule targets a model other than the current one (with the effort swap: when every class keeps the current effort).
- **Cache-aware switching** (`routing.cacheAware`, default `true`). Changing model discards the conversation's prompt cache. When a suggestion is cheaper per token but re-reading the cached context at its full input price would cost more than staying on the current model at the cached price, the router keeps the current model (`cache-sticky` in telemetry). Upgrades are never suppressed, and nothing is decided without prices for both models.
- **Prices** (`prices.enabled`, `prices.url`). Hosts often report no price (Claude Code) or zero (subscription models in OpenCode). The router then uses the [LiteLLM price table](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json), cached trimmed in `~/.local/state/system1router/model-prices.json` and refreshed by a detached background process when older than 7 days, so a request never waits for the download. `system1router prices [--refresh]` prints what each rule target costs. For subscription plans these are API list prices, used as a proxy for relative cost.

## Provider Setup

Provider credentials are referenced by environment variable name (`credentialEnv`); secret values are never included in diagnostics. `jev` and `laya` are optional and do not provide implicit fallback to one another.

Jev accepts `endpoint` for an exact decision endpoint or `baseUrl`/`JEV_BASE_URL` for deployment-provided URL configuration. Set the key separately:

```sh
export JEV_API_KEY='replace-with-your-key'
export JEV_BASE_URL='https://api.typesafe.ai/v1/systemone'
```

The native executable also loads these values from `~/.config/system1router/.env` when they are not already present in its environment:

```dotenv
# Replace the empty value with your Jev key.
JEV_API_KEY=
JEV_BASE_URL=https://api.typesafe.ai/v1/systemone
```

Process environment variables take precedence. An empty key is treated as unconfigured. Keep `.env` private and do not commit it.

```json
"providers": {
  "selected": "jev",
  "jev": {
    "baseUrl": "https://api.typesafe.ai/v1/systemone",
    "credentialEnv": "JEV_API_KEY",
    "model": "jev-latest"
  }
}
```

The native provider transport supports HTTP and HTTPS URLs. Requests are bounded, redacted, and sent with a bearer token using TypeSafe System One's `state`, `model`, and `questions` schema. The adapter maps the returned `answers` into a normalized `RoutingClassification`. Remote provider calls allow up to 3 seconds by default; hook execution allows up to 6 seconds.

Customize what System One receives under `providers.jev`. The default state places bounded conversation/project context before the **bounded, redacted** current request. `stateTemplate` replaces that order; `{query}` is required and `{context}` inserts bounded context. To change the classification criteria/instructions, `questions` replaces the entire TypeSafe questions object (it must contain `routing_class`, `task_type`, and `context`). Keep the six `routing_class` choice keys (`question_simple`, `question_moderate`, `question_hard`, `code_simple`, `code_moderate`, `code_hard`) and the same answer types so the reply remains routable. Example:

```json
"jev": {
  "baseUrl": "https://api.typesafe.ai/v1/systemone",
  "credentialEnv": "JEV_API_KEY",
  "stateTemplate": "Relevant conversation/project context: {context}\n\nCurrent request: {query}",
  "questions": {
    "routing_class": { "type": "choice", "instructions": "Choose the smallest suitable reasoning tier for this task.", "criteria": { "question_simple": "Direct answer", "question_moderate": "Multi-step answer", "question_hard": "Deep research", "code_simple": "Local edit", "code_moderate": "Multi-file edit", "code_hard": "Complex debugging or architecture" } },
    "task_type": { "type": "choice", "instructions": "Choose the primary intent.", "criteria": { "code-generation": "Writing code", "debugging": "Fixing code", "question-answering": "Answering", "analysis": "Analyzing", "planning": "Planning" } },
    "context": { "type": "score", "instructions": "How much project context is needed?", "criteria": ["Little", "Some", "Substantial"] }
  }
}
```

Laya supports a managed `command`, a local `endpoint`, a local `baseUrl`, or `LAYA_BASE_URL`:

```sh
export LAYA_BASE_URL='http://127.0.0.1:8788/classify'
```

Use `command` when System1Router should launch the local worker. Use `baseUrl` when another supervisor owns the worker. Both receive the same typed decision request, and neither enables remote fallback.

Telemetry is local and redacted. Set `telemetry.enabled` to `false` or `SYSTEM1ROUTER_TELEMETRY_DISABLED=1` to disable it. Debug capture requires both `telemetry.debugCapture: true` and `SYSTEM1ROUTER_DEBUG_CAPTURE=1`; even then only the bounded, redacted request projection is stored. Credentials, raw tool arguments, attachments, and model output are never captured by this path.

For provider troubleshooting, use `/s1router_config` in the OpenCode TUI to toggle verbose Jev exchange logging. The request log contains the bounded, redacted prompt projection but never the bearer token. Logs are stored in `~/.local/state/system1router/provider-debug.jsonl` (or `$XDG_STATE_HOME/system1router/provider-debug.jsonl`) and rotate after 2 MiB. Disable verbose logging after diagnosis. `/s1router_stats` reads local model usage and acceptance metadata, without storing prompts; the native core and OpenCode adapter share the same data contract so other hosts can contribute usage later.

Consent values are `interactive`, `always-accept`, `always-decline`, or `disabled`. Interactive one-shot hooks decline when no supported UI is available. Tests and host UIs may set `SYSTEM1ROUTER_CONSENT=accept` or `decline` for one request.

In OpenCode the model dialog also offers **Always switch for this pair** and **Never switch for this pair**. The answer is remembered for that exact current → suggested model/variant pair (in `~/.local/state/system1router/consent-preferences.json`) and later identical suggestions are applied or skipped without a dialog. `/s1router_config` lists remembered choices and can forget them.

The OpenCode confirmation uses `timeouts.consentMs` from the active config (JSON or TOML) for each request. It has no search field, highlights **Use suggested** by default, shows the time available when opened, and keeps the current model when the timer expires. The default is 15,000 ms. For two minutes, add `"timeouts": { "consentMs": 120000 }` to JSON, or `consentMs = 120000` under `[timeouts]` in TOML. The allowed range is 1–86,400,000 ms. Edits to the router config take effect on the next prompt.
