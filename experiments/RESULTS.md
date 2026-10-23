# Experiments

## 1. Does an effort change keep Claude Code's prompt cache?

**Question.** The Anthropic docs say a change of the request's `output_config.effort` invalidates the cached messages. If that held in Claude Code, every effort swap would cost a cache re-write, which is what a model swap costs.

**Method.** `experiments/effort_cache.sh <out> <model> [native|mod|both]` runs one conversation with `claude -p --resume`, one short turn each, and changes the effort between turns:

- **native**: the session's own effort (`--effort`, which is what `/effort` sets).
- **mod**: the System1Router mod rewrites `turn.step`'s effort. The session stays at `medium`, and the class is forced with `SYSTEM1ROUTER_JEV_RESPONSE` (`code_moderate` → high, `question_simple` → low, `code_hard` → xhigh).

The first turn carries the project docs (about 12k tokens), so the cached conversation is far larger than one turn. `experiments/effort_cache_report.py <out>` reads each turn's `usage` and takes the effort actually sent from the transcript's `api-request` log.

Setup: Claude Code 2.1.310, 2026-10-24. The whole experiment cost about $1.

**Results** (read% is the share of the prompt served from the cache):

| Run | Turn | Effort sent | Prompt | Cache read | Cache write | read% |
|---|---|---|---:|---:|---:|---:|
| Sonnet 5.5, native | 2 (same) | medium | 38,775 | 24,926 | 13,847 | 64% |
| | **3 (change)** | **high** | 39,271 | 36,983 | 2,286 | **94%** |
| | 4 (same) | high | 39,386 | 38,790 | 594 | 98% |
| | **5 (change)** | **low** | 39,501 | 39,286 | 213 | **99%** |
| | 7 (model → haiku) | low | 40,155 | 9,025 | 31,128 | 22% |
| Sonnet 5.5, mod | 2 (same) | high | 38,786 | 36,983 | 1,801 | 95% |
| | **3 (change)** | **low** | 39,271 | 36,983 | 2,286 | **94%** |
| | 4 (same) | low | 39,767 | 38,801 | 964 | 98% |
| | **5 (change)** | **xhigh** | 40,717 | 39,286 | 1,429 | **96%** |
| Opus 5.5, native | 2 (same) | medium | 38,199 | 24,901 | 13,296 | 65% |
| | **3 (change)** | **high** | 38,291 | 38,122 | 167 | **100%** |
| | 4 (same) | high | 38,383 | 38,214 | 167 | 100% |
| | **5 (change)** | **low** | 38,475 | 38,306 | 167 | **100%** |
| | 7 (model → haiku) | low | 38,736 | 9,025 | 29,709 | 23% |

**Findings.**

1. **An effort change keeps the cache in Claude Code.** This holds for the native `/effort` and for the mod's `turn.step` rewrite, on both Sonnet 5.5 and Opus 5.5: turns after a change read 94–100% of the prompt from the cache. A model switch reads 22–23%, and only because the global system-prompt cache (9,025 tokens) is shared across models.
2. **Why.** The request log shows Claude Code sends the effort twice: as `output_config.effort` on the request, and as `output_config` on a per-turn system message in `messages`. The second is the `per-turn-control-2026-07-01` beta, and the per-turn message is added only when the effort changes. That is the cache-preserving per-message effort the API docs describe. The mod's effort goes through the same path.
3. **Claude Code caches with a 1-hour TTL** (`"ttl": "1h"` on its cache breakpoints), not the API's 5-minute default.
4. **Artifact, not a finding.** Turn 2 re-writes about 13k tokens in every run, including the runs whose effort never changed. It comes from the first `--resume` re-rendering the session, not from effort.

**Consequences, applied on this branch.** Claude Code events carry `effortKeepsCache`, and the router never holds back an effort change there. The mod's dialog no longer warns about the cache. The cache guard (`effort.cacheGuardTokens`) stays for hosts that change the top-level effort.

**Not measured.**
- OpenCode, which sends the effort through the AI SDK's provider options; the docs say a top-level change invalidates the cache there.
- Haiku as the session model.
- Long interactive sessions without `--resume`.
