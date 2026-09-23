## 1. Native Runtime Foundation

- [x] 1.1 Establish the native executable workspace, package layout, supported OS/CPU targets, and versioned public interfaces.
- [x] 1.2 Define the normalized hook-event and decision schemas, protocol versioning, correlation IDs, capability levels, and documented exit codes.
- [x] 1.3 Define typed contracts for request envelopes, bounded context, model state, catalogs, the six canonical routing classes (`question_simple`, `question_moderate`, `question_hard`, `code_simple`, `code_moderate`, `code_hard`), recommendations, consent outcomes, gateway targets, and routing events.
- [x] 1.4 Add configuration loading and schema validation for providers, redaction, timeouts, consent modes, routing modes, gateway settings, rules, defaults, and fail-open behavior.
- [x] 1.5 Implement secret-safe configuration and capability diagnostics without credentials or raw request content.
- [x] 1.6 Record host API versions and capability findings for OpenCode, Claude Code, Codex, GitHub Copilot CLI/VS Code, and the generic hook contract.

## 2. Executable Hook Protocol And Installation

- [x] 2.1 Implement the one-shot executable that reads one JSON event from stdin and writes one JSON decision to stdout.
- [x] 2.2 Route logs and diagnostics exclusively to stderr and enforce bounded execution timeouts.
- [x] 2.3 Implement malformed-event, unsupported-version, startup-failure, timeout, and internal-error fail-open behavior.
- [x] 2.4 Implement host protocol translators for Claude Code, Codex, GitHub Copilot CLI/VS Code, and OpenCode’s plugin shim.
- [x] 2.5 Implement `init`, `uninstall`, and `doctor` commands with ownership markers, idempotent edits, version/hash tracking, and preservation of unrelated configuration.
- [x] 2.6 Add native binary packaging and installation instructions for supported OS/CPU targets.
- [x] 2.7 Benchmark one-shot startup and add an optional local daemon/socket mode only if measured hook latency justifies it.
- [x] 2.8 Add protocol fixtures and tests for every host input/output mapping and unsupported response field.

## 3. Request And Classification Core

- [x] 3.1 Implement request capture with correlation IDs, session/turn scope, recursion guards, immutable original-payload references, and eligibility/bypass checks.
- [x] 3.2 Implement bounded prompt projection and configurable secret/path/tool redaction for provider input.
- [x] 3.3 Implement the decision-provider interface and runtime validation for the six routing classes, task type, contextual signals, confidence, and provider metadata.
- [x] 3.4 Implement provider timeout, cancellation, malformed-response, low-confidence, and unavailable-provider outcomes as classification-unavailable states.
- [x] 3.5 Implement the Jev adapter using its documented typed decision API, credential configuration, endpoint configuration, and health diagnostics.
- [x] 3.6 Implement the local Laya adapter boundary with managed-process or configured-runtime behavior, health checks, and no implicit remote fallback.
- [x] 3.7 Add unit tests for all six routing classes, malformed responses, task-intent validation, confidence thresholds, context projection, redaction, recursion guards, and provider isolation.

## 4. Policy, Modes, And Consent

- [x] 4.1 Implement context-aware ordered policy matching across routing class, task type, request/project/session context, confidence, host/project/session scope, routing mode, and optional predicates.
- [x] 4.2 Implement host model-catalog validation and gateway upstream-allowlist validation.
- [x] 4.3 Implement deterministic first-match, default, original-model, and bypass resolution.
- [x] 4.4 Implement menu-style configuration examples for preferred models and variants by routing class, task category, and contextual predicate.
- [x] 4.5 Implement advisory, consent-only, full-override, and gateway capability/mode negotiation.
- [x] 4.6 Implement interactive consent with explicit `use suggested model` and `keep current model` choices, plus always-accept, always-decline, and disabled policies with timeout handling.
- [x] 4.7 Implement host response translation for allow, block, ask, and context-injection outcomes without claiming unsupported model changes.
- [x] 4.8 Add unit tests covering precedence, unavailable targets, mode fallback, defaults, unknown categories, consent timeout, and decline behavior.

## 5. Host Override And Restoration

- [x] 5.1 Implement adapter model-state snapshots containing provider, model, variant, and host/session scope.
- [x] 5.2 Implement accepted full-override execution with apply, unchanged request execution, and guaranteed restoration cleanup.
- [x] 5.3 Add per-session transaction isolation, serialization or request handles for concurrent overrides, and recursion protection.
- [x] 5.4 Implement restoration retries, verification, high-severity diagnostics, and safe behavior when apply fails before execution.
- [x] 5.5 Add fault-injection tests for request failure, cancellation, apply failure, restoration failure, and overlapping sessions.

## 6. Provider Gateway

- [x] 6.1 Choose and document the first supported gateway protocol, then define authentication, upstream allowlisting, and request/response compatibility rules.
- [x] 6.2 Implement the gateway endpoint and per-request classification/policy resolution without mutating host-global model state.
- [x] 6.3 Implement forwarding for supported streaming, tools, attachments, status codes, and error semantics.
- [x] 6.4 Implement explicit gateway enablement, client authentication, credential isolation, rate limits, and safe shutdown behavior.
- [x] 6.5 Add gateway tests for routing, fallback, unauthorized access, upstream failure, streaming, concurrent requests, and raw-payload redaction.
- [x] 6.6 Document client configuration for compatible endpoints and clearly identify hosts that cannot use gateway mode.

## 7. OpenCode Integration

- [x] 7.1 Implement the OpenCode TypeScript plugin shim that invokes the native executable through supported public lifecycle surfaces.
- [x] 7.2 Implement OpenCode model-catalog discovery and current provider/model/variant capture for the installed SDK version.
- [x] 7.3 Implement the OpenCode consent UI/notification path with an explicit suggested-model/current-model choice when a supported interactive TUI channel is available, and redacted routing diagnostics using supported public APIs.
- [x] 7.4 Implement the verified full-override path if the capability probe confirms one; otherwise expose observe-only or consent-only behavior without duplicate requests.
- [x] 7.5 Add OpenCode integration tests for normal requests, recursion guards, model availability, accept/decline, restoration, gateway configuration, unsupported capability, and plugin-disabled behavior.
- [x] 7.6 Add the OpenCode plugin/configuration entry and document installation, enablement, provider setup, gateway setup, and rollback.

## 8. Claude Code And Codex Integrations

- [x] 8.1 Implement Claude Code `UserPromptSubmit` command-hook installation and response translation through the executable protocol.
- [x] 8.2 Implement Claude Code `PreModelSwitch` capability detection and safe allow/deny behavior without pretending it initiates a switch.
- [x] 8.3 Implement Codex `UserPromptSubmit` command-hook installation and response translation through the executable protocol.
- [x] 8.4 Implement Codex tool-hook translation only for documented tool decisions and updated inputs, without treating it as model-request interception.
- [x] 8.5 Add Claude Code and Codex contract tests for advisory routing, fail-open, consent, diagnostics, gateway configuration, and unsupported full override.
- [x] 8.6 Document Claude Code and Codex installation, supported versions, trust requirements, limitations, and configuration.

## 9. GitHub Copilot, VS Code, And Generic Hooks

- [x] 9.1 Implement GitHub Copilot CLI `userPromptSubmitted`/`userPromptTransformed` adapter behavior using only documented response fields.
- [x] 9.2 Implement GitHub Copilot VS Code hook registration and response translation for supported user-prompt and tool events.
- [x] 9.3 Add a generic hook adapter example and protocol documentation for future clients with stdin/stdout command hooks.
- [x] 9.4 Add Copilot, VS Code, and generic-hook tests for capability reporting, provider availability, diagnostics, and safe bypass behavior.
- [x] 9.5 Document installation, host limitations, gateway applicability, and configuration for each supported Copilot surface.

## 10. Security, Telemetry, And Release Validation

- [x] 10.1 Implement local redacted routing-event telemetry with correlation ID, classification metadata, mode, policy outcome, latency, and error category.
- [x] 10.2 Add opt-in debug capture with redaction tests and verify credentials, prompts, attachments, tool arguments, and model output are excluded by default.
- [x] 10.3 Add end-to-end contract tests that run the same routing scenarios against every adapter capability level and gateway mode.
- [x] 10.4 Add documentation for policy precedence, fail-open/fail-closed behavior, privacy implications, provider/gateway data handling, and model-state recovery.
- [x] 10.5 Run the full test suite, type checks, native builds, packaging checks, installer tests, and host-specific smoke tests before enabling routing by default.
