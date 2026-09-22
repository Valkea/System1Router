## Why

The primary goal of System1Router is to provide a context-aware model router for AI coding clients. Given the current user query and the relevant bounded context, it should select the best available model, model tier, and variant for that request while considering task intent, conversation and project context, client capabilities, model availability, latency, cost, privacy, and user policy.

A lightweight System 1 decision provider such as Jev or Laya supplies the routing decision, but classification is only an intermediate step. The product value is the model recommendation and the safest host-native execution path for that recommendation. Hosts expose different levels of control and many cannot replace the primary model in-flight, so the router must distinguish actual model routing from advisory recommendations and native subtask delegation.

## What Changes

- Add a native, cross-platform `system1router` executable with a documented JSON stdin/stdout hook protocol, similar to RTK’s host installation model.
- Add an idempotent `init` command that installs and removes thin host-specific hook adapters without overwriting unrelated user configuration.
- Add a shared, context-aware routing core that selects the best available model for each eligible user query from the query, bounded session/project context, client capabilities, model catalog, and user policy.
- Add the canonical six-class routing taxonomy: `question_simple`, `question_moderate`, `question_hard`, `code_simple`, `code_moderate`, and `code_hard`, with task intent and contextual signals kept as separate routing inputs.
- Support pluggable decision-model providers, initially including hosted Jev and locally hosted Laya-compatible adapters.
- Resolve a configured target model and variant from the routing class, task intent, context, model catalog, and policy, then present the recommendation to the user before execution.
- Explicitly offer the user two choices for every eligible recommendation: use the suggested model/variant, or keep the model/variant already selected in the client.
- Apply an accepted model/variant override only through a host capability that explicitly supports it, and restore the exact prior selection after the request completes, including failures and cancellation.
- Add an optional provider gateway mode that routes each request to an upstream model per request, avoiding host-global model mutation where the client supports a compatible endpoint.
- Provide configuration for decision-model credentials/runtime, classification thresholds, routing policies, per-case preferred models, consent behavior, and fail-open behavior.
- Ship integrations in priority order: OpenCode plugin plus executable, Claude Code hooks, Codex hooks, GitHub Copilot CLI/VS Code hooks, and a documented generic hook contract for additional clients.
- Record explainable routing metadata locally without storing request content by default.

## Capabilities

### New Capabilities

- `request-interception`: Capture a request at the earliest supported lifecycle point and pass it through routing without changing its content or tools.
- `request-classification`: Classify the request into the configured six-class routing taxonomy and identify task intent and contextual signals with confidence metadata.
- `adaptive-model-routing`: Select the best available model and variant from the routing class, task intent, request context, model catalog, and policy.
- `routing-consent`: Show a concise recommendation and explicitly offer `use suggested model` or `keep current model` for the current request.
- `model-state-restoration`: Apply a temporary model/variant override and restore the exact prior state after success, failure, cancellation, or routing bypass.
- `hook-runtime`: Provide the native executable, normalized hook protocol, host protocol translators, installation, diagnostics, and fail-open behavior.
- `routing-gateway`: Route requests through a compatible provider gateway when native host hooks cannot change the model for the current request.
- `client-adapters`: Expose the common routing core through OpenCode, Claude Code, Codex, GitHub Copilot CLI/VS Code, and future hook-compatible clients with explicit capability levels.

### Modified Capabilities

<!-- No existing OpenSpec capabilities are present. -->

## Impact

- Adds a native executable distribution, shared JSON protocol, installer/configuration manager, provider gateway, decision-model adapters, and client-specific integration layers.
- Integrates with each client’s request lifecycle and model/variant selection APIs; where a client cannot mutate the current model, its hook remains advisory or delegates to the gateway.
- Adds runtime dependencies for the selected Jev transport and optional local Laya execution; these must remain optional so one unavailable provider does not disable routing.
- Adds optional upstream gateway credentials and endpoint configuration; gateway mode must never be enabled implicitly.
- Introduces local configuration and routing-event telemetry, with redaction and opt-out requirements.
