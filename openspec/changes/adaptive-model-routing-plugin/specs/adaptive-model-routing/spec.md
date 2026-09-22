## ADDED Requirements

### Requirement: Select the best available model from request context
The router SHALL resolve a recommendation for the best available model, model tier, and optional variant from the request query, bounded conversation/project context, routing class, task type, client capabilities, confidence, ordered policy rules, and model availability. A recommendation SHALL identify either an opaque host model reference or an allowed gateway upstream target and a human-readable reason.

#### Scenario: Context-aware routing rule matches
- **WHEN** a high-priority rule matches `code_hard`, `code-generation`, and the available project or client context
- **THEN** the router recommends the configured model and variant or gateway target for that rule
- **AND** the recommendation includes the rule ID, mode, and reason

#### Scenario: No specific rule matches
- **WHEN** no configured rule matches the classification
- **THEN** the router uses the configured default recommendation or bypasses routing
- **AND** it does not invent a model, variant, gateway target, or policy decision

### Requirement: Respect the selected routing mode
The router SHALL support advisory, host full-override, and gateway routing modes and SHALL only execute a mode when the current adapter or gateway satisfies that mode’s capability requirements.

#### Scenario: Host supports full override
- **WHEN** policy selects host full-override and the adapter reports `full-override`
- **THEN** the recommendation is eligible for consent and transactional execution

#### Scenario: Host supports only advisory hooks
- **WHEN** policy selects host full-override but the adapter reports `observe-only` or `consent-only`
- **THEN** the router falls back to advisory behavior or the configured gateway
- **AND** it does not mutate host state or resubmit the request

### Requirement: Respect the host model catalog and gateway allowlist
The router SHALL select a host target only when the host reports that the provider/model and requested variant are available, and SHALL select a gateway target only when it is present in the configured upstream allowlist.

#### Scenario: Recommended target is available
- **WHEN** the target model/variant or gateway upstream exists in the current catalog/allowlist
- **THEN** the recommendation is eligible for consent

#### Scenario: Recommended target is unavailable
- **WHEN** the target is absent from the host catalog or gateway allowlist
- **THEN** the router marks the recommendation unavailable
- **AND** the request remains on its original model and variant

### Requirement: Keep policy decisions deterministic
The router SHALL evaluate rules in a documented order, SHALL use the first eligible match, and SHALL treat a user’s per-request decline as a routing outcome rather than a persistent policy edit.

#### Scenario: Multiple context-aware rules match
- **WHEN** multiple rules match the same classification
- **THEN** the first eligible rule in configured priority order wins
- **AND** later rules are not applied

#### Scenario: User declines a recommendation
- **WHEN** the user declines the selected recommendation
- **THEN** the router executes the request using the captured original model and variant or the host’s normal path
- **AND** the configured policy remains unchanged
