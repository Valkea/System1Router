## ADDED Requirements

### Requirement: Provide a native executable hook protocol
The runtime SHALL provide a cross-platform executable that reads one versioned JSON event from stdin, writes one machine-readable JSON decision to stdout, writes diagnostics only to stderr, and exits within the configured timeout.

#### Scenario: Valid hook event is processed
- **WHEN** a host sends a valid normalized event to the executable
- **THEN** the executable returns one valid decision containing the correlation ID and routing outcome
- **AND** stdout contains no human-readable logs

#### Scenario: Malformed or unsupported event is received
- **WHEN** the executable receives malformed JSON or an unsupported protocol version
- **THEN** it returns a documented bypass/error result or exits with the documented fail-open code
- **AND** it does not emit a fabricated model recommendation

### Requirement: Support host protocol translation
The runtime SHALL support host-specific translators that map native hook input and output to the normalized protocol without changing unsupported host semantics.

#### Scenario: Hook response has unsupported fields
- **WHEN** the core produces a recommendation or action unavailable in the host protocol
- **THEN** the translator removes the unsupported fields
- **AND** it returns the closest safe bypass, ask, block, or context result supported by the host

### Requirement: Install hooks idempotently
The runtime SHALL provide install, uninstall, and diagnostic commands that add or remove only its own hook entries, preserve unrelated configuration, and record the installed executable/protocol version.

#### Scenario: Installation is repeated
- **WHEN** the user runs `system1router init --host <host>` twice
- **THEN** the host configuration contains one active system1router entry
- **AND** unrelated hook entries remain unchanged

#### Scenario: Installation is removed
- **WHEN** the user runs `system1router uninstall --host <host>`
- **THEN** only system1router-owned entries are removed
- **AND** the host remains usable with its other hooks

### Requirement: Fail open on executable failure
The runtime SHALL default to the host’s normal behavior when it cannot start, exceeds its timeout, loses its provider, or encounters an internal error, unless fail-closed mode is explicitly configured.

#### Scenario: Binary is missing during a hook event
- **WHEN** a configured host cannot execute the system1router binary
- **THEN** the host continues the original request
- **AND** the failure is visible in diagnostics without exposing request content

### Requirement: Support optional warm execution
The runtime MAY provide a local daemon or persistent process mode, but the one-shot stdin/stdout protocol SHALL remain functional and semantically equivalent.

#### Scenario: Daemon is unavailable
- **WHEN** warm execution is configured but the daemon is stopped or unreachable
- **THEN** the runtime falls back to one-shot execution or host bypass according to configuration
- **AND** it does not change routing semantics silently
