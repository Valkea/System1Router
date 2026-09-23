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

Variants are configured per target with `target.variant`; they are not a separate global setting. A host catalog may advertise allowed variant names through `models[].variants`. The router rejects a configured variant when the host explicitly reports that the variant is unavailable. OpenCode's current provider catalog does not expose variants through the plugin SDK, so configured variants are applied by name and OpenCode handles any unsupported variant response.

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

Customize what System One receives under `providers.jev`. `stateTemplate` replaces the default state string; `{query}` is required and is replaced with the **bounded, redacted** user prompt, while `{context}` inserts bounded context. To change the classification criteria/instructions, `questions` replaces the entire TypeSafe questions object (it must contain `routing_class`, `task_type`, and `context`). Keep the six `routing_class` choice keys (`question_simple`, `question_moderate`, `question_hard`, `code_simple`, `code_moderate`, `code_hard`) and the same answer types so the reply remains routable. Example:

```json
"jev": {
  "baseUrl": "https://api.typesafe.ai/v1/systemone",
  "credentialEnv": "JEV_API_KEY",
  "stateTemplate": "Route this coding request: {query}\nRelevant project context: {context}",
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
