## ADDED Requirements

### Requirement: Route compatible provider requests per request
The gateway SHALL accept a supported provider protocol, classify each eligible request before forwarding, resolve an allowed upstream model/variant, and forward the original request to that target without requiring host-global model mutation.

#### Scenario: Compatible request is routed
- **WHEN** a client sends a request through a configured gateway endpoint
- **THEN** the gateway classifies it, applies the configured policy and consent mode, and forwards it to the selected upstream target
- **AND** the upstream request preserves supported streaming, tools, attachments, and request semantics

#### Scenario: No gateway recommendation is eligible
- **WHEN** classification, consent, policy resolution, or upstream availability prevents routing
- **THEN** the gateway uses the configured fallback target or returns a documented fail-open response
- **AND** it does not select an unallowlisted upstream model

### Requirement: Preserve gateway response and error semantics
The gateway SHALL return upstream response formats, streaming behavior, status classes, and errors according to the configured client protocol, while adding only documented redacted routing metadata where supported.

#### Scenario: Upstream response streams
- **WHEN** the selected upstream returns a supported streaming response
- **THEN** the gateway forwards chunks in the client-compatible format
- **AND** it does not buffer or rewrite model content solely for routing

#### Scenario: Upstream request fails
- **WHEN** the selected upstream returns an error or times out
- **THEN** the gateway returns a protocol-compatible error
- **AND** the routing event records the failure without raw request or credential data

### Requirement: Secure and explicitly enable gateway mode
The gateway SHALL require explicit configuration, authenticate clients, restrict upstream targets to an allowlist, and keep credentials out of logs and hook responses.

#### Scenario: Gateway is not configured
- **WHEN** a client has no explicitly configured gateway endpoint
- **THEN** the host uses its normal provider path or advisory hook mode
- **AND** no request is silently sent to the gateway

#### Scenario: Unauthorized client connects
- **WHEN** a request lacks valid gateway authentication
- **THEN** the gateway rejects it before classification or upstream forwarding
- **AND** the rejection contains no secret material

### Requirement: Keep gateway routing stateless with respect to host model selection
Gateway execution SHALL not mutate or restore the client’s selected model or variant; the routed upstream target SHALL be scoped to the individual request.

#### Scenario: Two clients use different gateway routes
- **WHEN** concurrent requests resolve to different upstream targets
- **THEN** each request is forwarded independently
- **AND** one request cannot change another client’s visible model selection
