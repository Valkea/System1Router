## ADDED Requirements

### Requirement: Present an actionable model recommendation
The consent adapter SHALL present the recommended model, variant or gateway target, routing class, task type, relevant context summary, confidence summary, reason, and explicit accept/decline actions before a routed request executes when the host has an interactive consent channel.

#### Scenario: Interactive recommendation is displayed
- **WHEN** a valid recommendation is resolved for an eligible request and the host has an interactive consent channel
- **THEN** the adapter displays the recommendation before model execution
- **AND** the display includes the suggested model and the captured current model

### Requirement: Offer suggested or current model explicitly
For every eligible recommendation with an interactive consent channel, the adapter SHALL offer two mutually exclusive choices for the current request: `use suggested model` or `keep current model`. The `keep current model` choice SHALL mean the captured original provider, model, and variant, not a newly resolved fallback target.

#### Scenario: Interactive consent is available
- **WHEN** a valid recommendation is resolved for an eligible request
- **THEN** the adapter shows the recommendation before model execution
- **AND** the user can choose `use suggested model` or `keep current model`

#### Scenario: User keeps the current model
- **WHEN** the user chooses `keep current model`
- **THEN** the request executes with the captured original provider, model, and variant or the host’s normal path
- **AND** no model change or gateway route is executed

#### Scenario: User uses the suggested model
- **WHEN** the user chooses `use suggested model`
- **THEN** the router executes the recommendation through the adapter’s verified host override or configured routing path
- **AND** the choice applies only to the current request unless the user explicitly changes policy

#### Scenario: Recommendation is not eligible
- **WHEN** classification is unavailable or no eligible target exists
- **THEN** the adapter does not show a misleading model recommendation
- **AND** the request follows the configured fallback behavior

### Requirement: Translate consent through host-supported hook outputs
The hook runtime SHALL express advisory consent outcomes using only the host’s documented `allow`, `block`, `ask`, or context-injection fields and SHALL clearly label context injection as a recommendation rather than a model switch.

#### Scenario: Host supports an ask decision
- **WHEN** the user must choose before execution and the host supports an ask response
- **THEN** the adapter returns the host’s documented ask decision
- **AND** the host remains responsible for the final execution flow

#### Scenario: Host supports context injection but not model selection
- **WHEN** the user accepts a recommendation on a host with no model mutation field
- **THEN** the adapter may inject concise recommendation context if configured
- **AND** it does not claim that the selected model or variant changed

### Requirement: Honor the consent policy
The router SHALL support configured interactive, always-accept, always-decline, and disabled modes, with interactive mode as the default when a consent channel is available.

#### Scenario: User accepts interactively
- **WHEN** the user selects accept
- **THEN** the router proceeds with the eligible request-scoped host override or gateway route
- **AND** the acceptance applies only to the current request unless policy was explicitly changed

#### Scenario: User declines interactively
- **WHEN** the user selects decline
- **THEN** the router sends the original request using the pre-existing model and variant or normal host path
- **AND** no temporary host override is applied

### Requirement: Handle unavailable or expired consent safely
The consent operation SHALL have a configurable timeout and SHALL default to decline when the user interface is unavailable or the timeout expires, unless the user explicitly configures another non-interactive mode.

#### Scenario: Consent times out
- **WHEN** the user does not respond within the configured consent timeout
- **THEN** the request uses the original model and variant or normal host path
- **AND** the timeout is recorded as a distinct routing outcome
