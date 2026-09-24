## Context

The repository currently contains a small OpenCode plugin setup (`.opencode/plugins/`) and depends on `@opencode-ai/plugin` 1.4.3. It has no shared routing core, decision-model adapter, configuration schema, native executable, gateway, or cross-client integration layer.

The primary product is a context-aware model router, not a standalone request classifier. For each eligible user query, the router gathers the bounded context needed to understand the request, considers the available model catalog and host capabilities, selects the best model or model tier and variant under the active policy, and then applies that recommendation through the safest supported host path. The routing decision must account for task intent, conversation and project context, cost, latency, privacy, availability, and confidence.

The decision provider uses the canonical six-class routing taxonomy plus separate task-intent and context signals:

- `question_simple`: a direct question requiring limited reasoning or context.
- `question_moderate`: a question requiring multi-step reasoning or synthesis of relevant context.
- `question_hard`: a question requiring deep analysis, broad context, or difficult trade-off reasoning.
- `code_simple`: a small, localized, low-risk code change or implementation task.
- `code_moderate`: a multi-step, multi-file, or non-trivial implementation, debugging, or refactoring task.
- `code_hard`: a complex, high-risk, architectural, security, performance, or systemic debugging task.

These six class names and definitions are part of the canonical classification contract. They must not be collapsed into a generic difficulty scale or replaced by an unrelated taxonomy.

The target behavior spans request interception, context-aware routing, structured classification, interactive consent, model selection, and cleanup across hosts with different extension APIs.

RTK demonstrates a useful distribution pattern: one native executable, an `init` command that installs host-specific hooks, and small adapters that translate each host’s event protocol. Its interception target is shell tool input, however, and that is materially easier than changing an AI model request. The routing project must preserve this portability pattern while treating model switching as a capability rather than an assumption.

Current hook research shows that Claude Code, Codex, GitHub Copilot, and VS Code can invoke command hooks around user turns or tool calls, but their documented outputs generally block, add context, or modify tool input rather than select a different model. The installed OpenCode plugin API exposes `chat.message` and `chat.params`, but neither hook output currently provides a same-request model/variant mutation. A portable executable can therefore provide universal classification and recommendation, while true per-request model routing requires either a host-specific override API or a compatible provider gateway.

Jev is a hosted System One decision service that returns typed choices and confidence metadata. Laya is an optional local model/runtime that returns typed decisions without requiring request text to leave the machine. The routing decision must be treated as an advisory control-plane operation: the user request remains the data-plane payload and must still execute when routing is unavailable or declined.

## Goals / Non-Goals

**Goals:**

- Deliver a cross-platform native `system1router` executable with a stable JSON stdin/stdout protocol.
- Install, inspect, and remove host hooks through an idempotent command similar to RTK’s `init` workflow.
- Define one host-independent routing state machine and typed request/decision/recommendation contracts.
- Classify each eligible user turn into exactly one of the six configured routing classes, identify task intent and relevant contextual signals, and return confidence and provider metadata.
- Support hosted Jev and optional local Laya providers behind the same decision-provider interface.
- Resolve deterministic model/variant recommendations from user-configured policy rules and available model catalogs.
- Provide advisory hook mode for broad compatibility and provider-gateway mode for true per-request routing where a compatible endpoint is available.
- Change the model/variant only for the current request through a verified request-scoped `full-override` capability.
- Make privacy, timeouts, failures, unsupported host capabilities, and concurrency explicit and fail open.
- Deliver integrations in priority order: OpenCode, Claude Code, Codex, GitHub Copilot CLI/VS Code, then additional generic hook-compatible clients.

**Non-Goals:**

- Building or fine-tuning a new System One model.
- Automatically changing the user’s persistent default model policy without explicit configuration.
- Rewriting prompts, tools, attachments, conversation history, or model responses in advisory mode.
- Circumventing a client’s extension or permission model to force a model swap.
- Promising full model switching for a host whose documented API exposes only observation, blocking, context injection, or tool-input mutation.
- Making gateway mode mandatory or silently rerouting requests through an external endpoint.
- Storing raw prompts, full conversations, credentials, or model output in routing telemetry by default.

## Decisions

### Native executable with a normalized hook protocol

The portable core will be distributed as a native binary, preferably implemented in Rust to match the single-binary, low-dependency pattern proven by RTK. The primary command protocol is one JSON event on stdin and one JSON decision on stdout; diagnostics and logs go to stderr. The protocol includes:

- `protocolVersion`, `host`, `event`, `sessionId`, `turnId`, and `cwd`;
- bounded request content and host metadata;
- current model/variant when the host exposes it;
- capability hints and a correlation ID; and
- a response of `allow`, `block`, `ask`, `bypass`, or `route`, plus optional host-neutral recommendation metadata.

The executable will never emit human-readable logs on stdout because hosts parse that stream. A malformed event, timeout, crash, or unsupported protocol defaults to the host’s normal behavior unless fail-closed mode was explicitly configured.

### Thin host adapters and idempotent installation

`system1router init --host <name>` will install the smallest supported adapter for the selected host: a command-hook entry for Claude Code, Codex, Copilot, or VS Code, and a TypeScript plugin shim for OpenCode where required. Installation must preserve unrelated entries, be repeatable, record the installed version/hash, and support `uninstall` and `doctor` commands. Host adapters translate native event schemas into the normalized executable protocol and translate its response back into the host’s documented response schema.

The executable owns context projection, classification, policy, consent-independent decisions, redaction, telemetry, and gateway coordination. Host adapters own only lifecycle registration and UI/response translation.

### Explicit capability modes

Each adapter reports one of:

- `observe-only`: classify and record, without affecting the request;
- `consent-only`: present or inject a recommendation, but cannot apply a model override;
- `full-override`: safely selects a model/variant on the current request without changing persistent client selection; or
- `gateway`: sends the request through a configured routing endpoint that selects the upstream model per request.

The mode is detected and displayed by `doctor`. The core must never simulate full override by recursively submitting the prompt, mutating a future turn, or changing persistent/global model selection.

### Provider gateway for true per-request routing

The optional gateway exposes only provider-compatible endpoints that the selected client can use. It receives the original request, classifies it before forwarding, resolves an upstream model/variant, applies consent or policy, and forwards the original request to that upstream target. It must preserve streaming, tools, attachments, and response/error semantics supported by the chosen protocol.

Gateway routing is per-request and never mutates persistent host model selection. The gateway must authenticate callers, restrict allowed upstream targets, avoid logging raw payloads, and fail according to explicit policy. Clients that cannot configure a compatible endpoint remain in advisory or host-adapter mode.

### Shared core with host adapters

The core will expose pure routing operations and a small adapter interface:

- `capture(request) -> RequestEnvelope`
- `classify(envelope) -> RoutingClassification`
- `recommend(classification, context, catalog, policy) -> Recommendation | bypass`
- `requestConsent(recommendation, context) -> accept | decline | unavailable`
- `forwardThroughGateway(request, recommendation) -> response`

Adapters own lifecycle hooks, model catalog discovery, current-state access, consent UI, model mutation, and protocol translation. This keeps provider and policy tests independent of host SDKs and prevents one client’s limitations from leaking into another.

### Structured provider contract

Decision providers will implement a common contract accepting a bounded, redaction-aware envelope and returning validated structured data. The canonical result contains one value from the six-class `routingClass` taxonomy, `taskType`, relevant context signals, confidence per dimension, provider/version metadata, and a correlation ID. Jev requests will use its typed Choice/Score primitives where available. Laya will run through its supported local Python/runtime boundary rather than being bundled into the native executable.

Provider responses will be validated against a local schema. Invalid output, timeout, unavailable credentials, low confidence, or provider error produces `classification-unavailable`, not a guessed route. The provider timeout is bounded and shorter than the host’s normal request timeout.

### Policy-driven recommendation resolution

Configuration will define ordered rules over routing class, task type, contextual predicates, confidence range, host/project/session scope, routing mode, and model availability. A rule resolves to an opaque host model reference or gateway upstream target, an optional variant, and a human-readable reason. First matching rule wins; an explicit default and the current selection provide deterministic fallbacks. The resolver will reject targets absent from the host catalog or gateway allowlist and will never route to a fabricated model ID.

The configuration format will support a menu-like matrix so users can favor a model for each routing-class/task/context case without writing code. Policy changes affect future requests only and are not modified by an individual consent decision.

### Consent as an adapter/protocol operation

The core will request one decision per routed request. When a recommendation is eligible, an interactive adapter must explicitly offer two mutually exclusive choices: `use suggested model` or `keep current model`. It displays the classification summary, recommendation, confidence, reason, and the effect of each choice. Choosing `keep current model` is the safe decline path: it executes the request with the captured original selection and does not change the request model or route it through the gateway. A command hook may express the result through the host’s supported `allow`, `block`, `ask`, or context-injection response, but it must not claim that context injection changed the model. Default behavior for a missing or non-interactive consent channel is decline, unless an explicit non-interactive policy is configured.

### Request-scoped host override and stateless gateway execution

For `full-override` adapters, an accepted recommendation changes only the model/variant attached to the original request. The adapter must use a request-scoped host API; it must not alter persistent session or user model selection. Adapters that can only change persistent selection remain advisory or use the gateway.

For `gateway` mode, the gateway forwards one request to the selected upstream target and performs no host-state mutation. Both modes are scoped to a single request.

### Privacy and local telemetry

The provider and gateway boundaries receive only configured request fields, with optional truncation/redaction of paths, secrets, file contents, and tool arguments. Routing events store correlation ID, host, classification labels/confidences, selected policy rule, mode, outcome, latency, and error category. Raw request text and credentials are excluded by default; debug payload capture requires explicit opt-in and redaction.

## Risks / Trade-offs

- [Hook capability gap] Most hosts cannot change the model from a hook → provide advisory mode broadly and gateway/full-override mode only where verified.
- [Gateway compatibility] A client may not support a configurable compatible endpoint or may use proprietary authentication → keep gateway optional and retain host-specific/advisory adapters.
- [Decision latency] Every request adds a classifier call and potentially a consent prompt → enforce short timeouts, support bypass rules, and add a warm daemon mode after the one-shot protocol is stable.
- [Native distribution] Multiple OS/CPU binaries and code signing increase release work → use reproducible builds, a small dependency footprint, and package-manager/direct-download installation.
- [Hook protocol breakage] Host schemas differ and evolve → version the normalized protocol, keep translators thin, and add fixture tests per host/version.
- [Classifier error] A wrong routing class or contextual signal can select a weaker or more expensive model → require confidence thresholds, configurable safe defaults, explicit consent, and a one-click decline path.
- [Provider availability] Jev credentials/network or Laya runtime may be unavailable → make providers optional and fail open to the pre-existing model.
- [Persistent model changes] A host may expose only session-level model mutation → do not advertise full override for that host; use advisory or gateway routing.
- [Sensitive prompt exposure] Classification or gateway forwarding may send user content to external services → prefer local Laya where configured, redact/truncate inputs, document provider data handling, and provide hard disable switches.

## Migration Plan

1. Add the native executable, normalized protocol, and configuration with routing disabled by default; existing client behavior remains unchanged.
2. Add `init`, `uninstall`, and `doctor` for Claude Code and Codex command hooks, then add Copilot/VS Code and the OpenCode plugin shim.
3. Validate every adapter in observe-only mode against recorded event fixtures before enabling consent or routing responses.
4. Add Jev and Laya providers independently, with configuration diagnostics and no mandatory dependency on either runtime.
5. Enable consent-based advisory routing for selected projects.
6. Add gateway mode for compatible endpoints and enable full override only after a host capability probe confirms safe same-request mutation.
7. Expand policy coverage gradually while collecting redacted diagnostics.
8. Roll back by disabling/removing the installed hook or gateway configuration; the fallback path sends requests through the original client model and removes no unrelated user configuration.

## Open Questions

- Should the native executable use a one-shot process for the first release and add a local daemon only after latency measurements justify it?
- Which gateway protocols should be supported first: OpenAI-compatible, Anthropic-compatible, or both?
- What Jev API endpoint, authentication flow, request schema, rate limits, and data-retention terms should the first adapter target?
- Should the first Laya integration launch a managed local worker, require a user-provided process, or support both?
- Which OpenCode release/API will provide a supported same-request model and variant override, if any, and can it be exposed without re-submitting the prompt?
- Which host event schemas and versions should be pinned for Claude Code, Codex, Copilot CLI, and VS Code?
- Should a low-confidence result be shown to the user as a recommendation, or bypass consent entirely and use the current model?
