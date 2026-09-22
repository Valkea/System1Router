## ADDED Requirements

### Requirement: Capture supported user-turn events before model execution
The routing layer SHALL receive each supported host user-turn event before the host starts processing that turn and SHALL create a request envelope containing a correlation ID, host/session/turn scope, current model state when available, bounded query content, relevant bounded conversation/project/client context, and an immutable reference to the original payload.

#### Scenario: Supported hook event is captured
- **WHEN** a configured host emits an eligible user-turn event
- **THEN** the adapter creates exactly one routing attempt for that event before model execution
- **AND** the original request payload and tool configuration remain unchanged

#### Scenario: Host only exposes tool hooks
- **WHEN** a host has no user-turn or model-request hook and only exposes tool events
- **THEN** the adapter reports tool-only coverage
- **AND** it does not claim to intercept or classify every model request

### Requirement: Normalize and translate host hook protocols
The adapter SHALL translate host-specific hook input into the versioned executable protocol and SHALL translate the executable decision back into only the response fields supported by that host.

#### Scenario: Claude Code event is normalized
- **WHEN** Claude Code invokes a `UserPromptSubmit` command hook
- **THEN** the adapter forwards a normalized event containing the prompt, session scope, and host identifier
- **AND** it maps the result to Claude Code’s documented hook output format

#### Scenario: Host lacks a response field
- **WHEN** the executable returns recommendation metadata that the host cannot represent
- **THEN** the adapter omits that unsupported field
- **AND** it does not emulate model switching by submitting another request

### Requirement: Fail open when interception cannot route
The routing layer SHALL send the original request through the host’s pre-existing model selection when classification, consent, policy resolution, executable startup, or host capability is unavailable, unless the user explicitly configured a fail-closed policy.

#### Scenario: Decision provider times out
- **WHEN** classification exceeds the configured timeout
- **THEN** the request continues with its original model and variant
- **AND** the timeout is recorded as a routing outcome without blocking the request

#### Scenario: Host lacks an override capability
- **WHEN** the adapter reports that it cannot apply a per-request model override and no gateway is configured
- **THEN** the request continues unchanged with the original selection
- **AND** the user-facing diagnostic states that adaptive routing was bypassed

### Requirement: Preserve request data-plane behavior
Advisory interception SHALL never rewrite the user’s prompt, conversation history, attachments, tool definitions, or tool arguments as part of classification or model selection.

#### Scenario: Request contains tools and attachments
- **WHEN** a request with tools or attachments is routed in advisory mode
- **THEN** classification uses only the configured bounded representation
- **AND** the executed request retains the original tools, attachments, and arguments byte-for-byte where the host supports such comparison
