# Operations and Safety

Policy precedence is deterministic: lower numeric `priority` first, then declaration order; the first target that passes confidence, scope, capability, catalog, and gateway allowlist checks wins. A declined recommendation is a per-request outcome and never edits policy.

Classification, consent, startup, timeout, and unsupported capability failures fail open by default. Set `routing.failOpen` to `false` only when the host’s documented contract can safely represent a block. Even in fail-open mode, no fabricated model target is emitted.

Advisory and consent-only adapters do not mutate model state or resubmit prompts. Full override uses a session-scoped snapshot, apply, unchanged request, and restoration cleanup. Restoration retries are bounded; a failure produces a high-severity diagnostic with host/session scope and last known model state, never prompt content or credentials.

Gateway mode is opt-in, requires a configured auth token and upstream allowlist, and is stateless with respect to the client’s selected model. Client authentication is not forwarded upstream. Use a TLS reverse proxy for remote HTTPS upstreams when the stdlib gateway is used.
