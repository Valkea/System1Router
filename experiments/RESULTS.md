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

## 2. Jev against small local encoders, with and without the conversation

**Question.** How well does Jev pick the effort, how would a small local classifier compare, and does the classification use the conversation? A short prompt ("ok do it", "why?") is often a reference to a complex earlier problem.

**Data.**
- `experiments/prompts.jsonl`: 150 synthetic coding-assistant prompts, 25 per class, without conversation.
- `experiments/followups.jsonl`: 20 **pairs**. Each pair is the same short follow-up ("ok do it", "same for the other one", "why?", "continue", "go"…) after two conversations, one that makes it simple and one that makes it hard. Each conversation carries assistant text, tool calls, tool results, files and tools, rendered as the router renders them for Jev. A classifier that ignores the conversation can get at most one of each pair right.

All prompts are written and labelled by one person (the author of this experiment). Scores measure agreement with that labeller on that style of prompt, not ground truth. No user data was used; only these synthetic prompts were sent to Jev.

**Method.** `experiments/classifier_bench.py`:
- **Jev:** each item goes through the branch binary. The context goes in the event, so it reaches Jev's `state` exactly as in production.
- **Local:** ONNX encoders on CPU via `onnxruntime`, with a logistic head under repeated 5-fold CV. On the follow-ups, whole pairs are held out and the head also trains on the 150 base prompts.

Run on 2026-10-24 against `jev-1.14.0` (what `jev-latest` resolves to today).

**Base prompts** (effort is the class mapped to `effort.levels`; under/over is an effort below/above the label's):

| Classifier | class | code/question | effort | under | over | latency |
|---|---:|---:|---:|---:|---:|---|
| always `code_moderate` | 17% | 50% | 33% | 17% | 50% | — |
| Jev, argmax | 82% | 100% | 82% | 17% | 1% | 247 ms median, 328 ms p95 (process + HTTPS) |
| Jev, median tier (current default) | 81% | 100% | 81% | 18% | 1% | same |
| **Jev, upper tier (20%)** | **94%** | 100% | **94%** | **3%** | 3% | same |
| MiniLM-L6 (22M) + head | 62% | 87% | 63% | 19% | 18% | 49 ms (1 thread) / 19 ms (4); load 235 ms; 90 MB |
| bge-small (33M) + head | 61% | 90% | 62% | 20% | 18% | 19 / 10 ms; load 373 ms; 133 MB |
| arctic-embed-xs (22M) + head | 59% | 83% | 60% | 21% | 19% | 9.5 / 4.8 ms; load 210 ms; 90 MB |
| ModernBERT-embed-base (149M) + head | 68% | 94% | 69% | 14% | 16% | 98 / 55 ms; load 1.5 s (14 s cold); 596 MB |
| same heads trained on Jev's labels | 51–58% | 85–92% | 53–59% | 34–38% | 5–12% | |

**Follow-up pairs** (pairs = both of a pair right):

| Classifier | class | pairs | effort | under | over |
|---|---:|---:|---:|---:|---:|
| Jev, prompt only | 38% | 0% | 38% | 57% | 5% |
| Jev, prompt + conversation, argmax | 62% | 30% | 62% | 35% | 2% |
| **Jev, prompt + conversation, upper tier (20%)** | **72%** | **45%** | **75%** | **20%** | 5% |
| Jev, + follow-up instructions (`jev-questions-followups.json`), upper tier | 75% | 55% | 78% | 18% | 5% |
| local heads, prompt only | 28–40% | 0% | 28–42% | 38–52% | 15–30% |
| local heads, request + newest conversation as one text | 68–78% | 45–55% | 68–78% | 10–18% | 10–22% |
| local heads, [prompt embedding ; conversation embedding] | 48–60% | 20–35% | 48–60% | 22–28% | 12–25% |

**Findings.**

1. **The conversation is essential.** Without it no classifier gets a single pair right, and Jev under-rates 57% of follow-ups. Production already sends it: the Claude Code hook and mod read the transcript tail, OpenCode sends recent messages. With it, Jev rises from 38% to 62% (argmax). Only this bench lacked it at first.
2. **Jev is calibrated one tier low against these labels.** Its errors are almost all one tier below: hard → moderate, moderate → simple. Its probabilities hold the right tier, though. Picking the **hardest tier whose upper tail holds ≥ 20% of the probability mass** lifts the base set from 82% to 94% and cuts under-effort from 17% to 3%. On follow-ups it goes from 62% to 72%, with under-effort from 35% to 20%. This is a cheap change, but it was tuned on 190 items from one labeller, so check it on real prompts before trusting the number.
3. **Custom instructions barely move Jev.** Telling it that follow-ups inherit the referenced task's difficulty changed argmax by −2 points and upper-tier by +3 points: noise at this size.
4. **Small local encoders are fast, but no match yet.** Encoding takes 5–50 ms, but loading the model takes 200–400 ms per process. The router spawns a process per prompt, so a local model needs a long-lived worker: a daemon, or the mod keeping it alive.
   - With 120 training labels, frozen heads reach 59–68% on the base set, against Jev's 82–94%.
   - Learning from Jev's labels (distillation) did worse (51–58%), because Jev and the labeller disagree on tiers.
   - With the conversation as one text (request first), heads reach 68–78% on follow-ups. That is optimistic, since they trained on pairs of the same construction.
   - Concatenating separate prompt and conversation embeddings is worse than one joint text.
5. **Train later, from real usage.** A local model needs thousands of labelled real prompts with their context, not a few hundred synthetic ones. Jev (with upper-tier selection) can label them as a teacher, and user overrides can correct it.

**Not measured.**
- Real prompts.
- Other labellers (inter-rater agreement).
- Fine-tuned encoders: these heads were frozen.
- Ettin and mmBERT: they have no embedding-tuned ONNX export.
- `granite-embedding-small-english-r2`: no ONNX export on the Hub.
