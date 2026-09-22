## ADDED Requirements

### Requirement: Snapshot and restore exact host model state
Before applying an accepted recommendation through a `full-override` adapter, the router SHALL snapshot the exact provider, model, variant, and host/session scope required to restore the prior selection, and SHALL attempt restoration after every terminal request outcome.

#### Scenario: Full-override request succeeds
- **WHEN** an accepted host override is applied and the model request succeeds
- **THEN** the router restores the captured provider, model, variant, and scope before completing the transaction

#### Scenario: Full-override request fails or is cancelled
- **WHEN** the overridden model request fails, is interrupted, or is cancelled
- **THEN** restoration runs through the cleanup path
- **AND** the original state is restored or a restoration failure is surfaced as an actionable diagnostic

### Requirement: Do not mutate state on advisory or gateway routing
The router SHALL not apply or restore a temporary host override when the request uses advisory mode, gateway mode, consent decline, classification failure, policy bypass, or a host without `full-override` capability.

#### Scenario: Gateway routes a request
- **WHEN** an accepted recommendation is executed through the gateway
- **THEN** the gateway selects an upstream target for that request
- **AND** no host model state is changed or restored

#### Scenario: User declines recommendation
- **WHEN** consent returns decline
- **THEN** the host selection remains exactly as captured
- **AND** no restoration transaction is created

### Requirement: Isolate concurrent host transactions
The router SHALL associate each host override transaction with its request/session scope and SHALL prevent restoration from one request from changing another request’s active model selection.

#### Scenario: Two sessions route concurrently
- **WHEN** two independent sessions accept different host overrides at overlapping times
- **THEN** each transaction uses its own snapshot and cleanup boundary
- **AND** one session’s restoration does not overwrite the other session’s selection

### Requirement: Surface restoration failure
If exact host restoration cannot be confirmed, the adapter SHALL emit a high-severity diagnostic containing the host/session scope and last known model state without exposing request content or credentials.

#### Scenario: Host rejects restoration
- **WHEN** the host rejects the restoration operation
- **THEN** the router retries according to configuration and records the failure
- **AND** the user receives a diagnostic that the model state may require manual correction
