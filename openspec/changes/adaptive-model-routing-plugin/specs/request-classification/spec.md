## ADDED Requirements

### Requirement: Return a validated six-class routing classification
The decision provider SHALL return a validated result with exactly one `routingClass` value from this canonical enum: `question_simple`, `question_moderate`, `question_hard`, `code_simple`, `code_moderate`, or `code_hard`. It SHALL also return task intent from the configured taxonomy or `other`, relevant bounded context signals, confidence values in the inclusive range `0..1`, provider metadata, and a correlation ID. Task intent is secondary metadata; it SHALL NOT replace or redefine the six routing classes.

The class definitions SHALL be:

- `question_simple`: a direct question requiring limited reasoning or context.
- `question_moderate`: a question requiring multi-step reasoning or synthesis of relevant context.
- `question_hard`: a question requiring deep analysis, broad context, or difficult trade-off reasoning.
- `code_simple`: a small, localized, low-risk code change or implementation task.
- `code_moderate`: a multi-step, multi-file, or non-trivial implementation, debugging, or refactoring task.
- `code_hard`: a complex, high-risk, architectural, security, performance, or systemic debugging task.

#### Scenario: Provider returns a valid six-class classification
- **WHEN** the provider evaluates a request successfully
- **THEN** the core accepts the result only after schema validation
- **AND** the result includes routing class, task type, contextual signals, confidence, provider identity, and model/version metadata
- **AND** the routing class is one of the six canonical enum values

#### Scenario: Provider returns malformed output
- **WHEN** the provider response omits a required field or contains an invalid enum/confidence
- **THEN** the core marks classification unavailable
- **AND** no model recommendation is applied

### Requirement: Support configurable task categories
The classification contract SHALL support at least `code-generation`, `question`, `explanation`, `debugging`, `refactoring`, `code-review`, `planning`, and `other`, while allowing users to configure additional categories supported by the selected provider.

#### Scenario: Code-generation request is classified in context
- **WHEN** a request asks the client to create code
- **THEN** the structured result can identify `code-generation` independently of its routing class
- **AND** the routing class remains a separate decision dimension from task type

#### Scenario: Unknown task category is returned
- **WHEN** a provider returns a category not enabled in configuration
- **THEN** the core maps it to `other` or marks the result unavailable according to the configured validation policy
- **AND** it does not silently select an unconfigured route

### Requirement: Bound and protect provider input
The classification request SHALL apply configured truncation and redaction rules before sending data to a remote provider, SHALL exclude credentials, and SHALL keep raw request content out of routing logs by default.

#### Scenario: Prompt contains a detected secret
- **WHEN** a request includes a token matching a configured secret pattern
- **THEN** the provider input replaces the token with a redaction marker
- **AND** the routing event contains no raw token or full prompt

#### Scenario: Local provider is selected
- **WHEN** the user selects a local Laya provider
- **THEN** the request remains on the local provider boundary
- **AND** the core records provider metadata without sending the prompt to Jev

### Requirement: Treat uncertainty as a routing input
The core SHALL expose classification confidence and SHALL allow policy rules to require minimum confidence for a recommendation.

#### Scenario: Confidence is below policy threshold
- **WHEN** a valid classification has confidence below the rule threshold
- **THEN** the resolver does not select that rule
- **AND** routing falls through to the next matching rule or the original model
