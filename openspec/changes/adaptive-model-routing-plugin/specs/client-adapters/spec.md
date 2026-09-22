## ADDED Requirements

### Requirement: Implement a common adapter contract
Each client adapter SHALL expose lifecycle registration, request normalization, current model-state capture when available, model catalog discovery when available, consent translation, temporary override when supported, restoration when supported, capability reporting, installation metadata, and redacted diagnostics through the shared core contract.

#### Scenario: Adapter passes contract validation
- **WHEN** an adapter is loaded for a supported host
- **THEN** it reports its host identity, supported capability level, protocol version, and implementation version
- **AND** the core can execute the same routing state machine without host-specific branching

#### Scenario: Adapter is unavailable
- **WHEN** a host integration cannot load or its capability probe fails
- **THEN** the plugin remains disabled for that host
- **AND** it does not intercept or modify the host’s request flow

### Requirement: Support the portable host installation order
The project SHALL deliver and document integrations in this order: OpenCode plugin/executable shim, Claude Code hooks, Codex hooks, GitHub Copilot CLI/VS Code hooks, and a generic normalized hook contract for additional clients. Each adapter SHALL document host versions and lifecycle/model-selection features required for each capability level.

#### Scenario: OpenCode adapter is installed
- **WHEN** the plugin runs in OpenCode
- **THEN** the OpenCode adapter is selected before lower-priority adapters
- **AND** it uses supported public plugin/SDK surfaces for the installed version

#### Scenario: Codex command hook is installed
- **WHEN** the user runs `system1router init --host codex`
- **THEN** the installer adds an idempotent `UserPromptSubmit` hook using the executable protocol
- **AND** existing Codex hook configuration remains intact

#### Scenario: Host supports observation but not override
- **WHEN** a client exposes an incoming-request hook but no safe current-request model mutation
- **THEN** the adapter reports `observe-only` or `consent-only`
- **AND** the core bypasses model swapping instead of duplicating, rewriting, or recursively submitting the request

### Requirement: Keep provider and host integrations optional
The client runtime SHALL load without Jev credentials, without a local Laya runtime, and without any non-selected host integration, and SHALL allow the user to select, configure, or disable each provider independently.

#### Scenario: Jev is not configured
- **WHEN** no Jev credential or endpoint is available
- **THEN** the Jev adapter is unavailable
- **AND** the host continues to operate with another configured provider or the original model

#### Scenario: Laya runtime is not installed
- **WHEN** local Laya cannot be started or contacted
- **THEN** the Laya adapter reports a provider error
- **AND** the request follows the configured fail-open path without loading remote fallback implicitly

### Requirement: Expose safe configuration and diagnostics
Adapters SHALL load configuration from documented host-appropriate locations, SHALL keep secrets in host-supported secret storage or environment variables, and SHALL provide a diagnostic view of installation status, capability, provider health, active policy, gateway status, and last routing outcome without raw request content.

#### Scenario: User inspects routing diagnostics
- **WHEN** the user runs the diagnostic command
- **THEN** the output includes installed hosts, capability levels, provider availability, policy status, and redacted recent outcomes
- **AND** it excludes credentials and raw prompts by default
