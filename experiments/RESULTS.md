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

## 3. A classifier trained on past sessions

**Question.** Can past sessions train a local classifier that picks the effort better than Jev?

**Data.**
- `experiments/dataset_extract.py` took every prompt the person typed in Claude Code (948) and OpenCode (1,587), 2,535 in total across 21 projects. Each one comes with its context and with what the turn then took: tool calls, files edited, tokens, time, interruption, and the next prompt.
- Opus 5.5 judges labelled the effort each prompt needed, in hindsight from that outcome, following `experiments/judge_rubric.md`. Shards 00–13 ran at effort xhigh and 14–25 at effort high; the two batches gave the same label distribution.
- Label mix: low 31%, medium 35%, high 26%, xhigh 9%, max under 0.1%. The person ran at `xhigh` or `max` about half the time.
- The dataset stays in `$XDG_STATE_HOME/system1router/dataset` (mode 0600), never in the repository.

**Method.**
- `experiments/train_classifier.py` fine-tunes `jhu-clsp/mmBERT-small` on CPU, with frozen token embeddings, 4 epochs, batch 8, 256 tokens. The input is the request, then the newest context.
- A fifth of the sessions are held out: 509 prompts from 96 sessions the model never sees in training.
- `experiments/jev_on_dataset.py` asks Jev (`jev-1.14.0`) for the same 509 prompts with their context, with the person's consent.

**Results on the 509 held-out prompts:**

| Classifier | exact | ±1 | under | over | MAE | latency |
|---|---:|---:|---:|---:|---:|---|
| the effort the turn ran at | 23% | 65% | 12% | 65% | 1.28 | — |
| always `medium` | 33% | 91% | 35% | 32% | 0.76 | — |
| Jev, argmax → `effort.levels` | 44% | 95% | 22% | 34% | 0.60 | 183 ms median (network) |
| Jev, upper 20% → `effort.levels` | 45% | 90% | 10% | 45% | 0.66 | same |
| ceiling of `effort.levels` (the judge's own class, mapped) | 59% | 100% | 3% | 39% | 0.41 | — |
| **mmBERT-small fine-tuned, argmax** | **59%** | **95%** | 23% | **18%** | **0.47** | 172 ms (4 threads), 405 ms (1); load 370 ms; 565 MB |
| mmBERT-small fine-tuned, upper 20% | 56% | 94% | 15% | 29% | 0.51 | same |
| same, trained on 1,400 labels (different test split) | 55% | 92% | 29% | 15% | 0.53 | |

**Findings.**

1. **The person's default effort overspends.** It was above what the turn needed 65% of the time.
2. **The fine-tuned model beats Jev on every measure but one.**
   - Against Jev's argmax: +15 points exact, 0.47 against 0.60 MAE, and about half the over-efforts.
   - With upper-20% selection it keeps under-effort at 15% (Jev: 10%) while over-spending far less (29% against 45%).
   - Going from 1,400 to 2,535 labels improved it (55% → 59%, under 29% → 23%), so more labels should still help.
3. **The class → effort table is itself a ceiling.** Even with the judge's own class, `effort.levels` reaches only 59% exact. Predicting the effort directly is the better target, whatever the classifier.
4. **Bias to keep in mind.** The model learned the same judge it is scored against, while Jev was not trained on that judge. The gap is real, but partly a matter of agreeing with this judge.
5. **Latency is no better than Jev's yet.**
   - The model runs in 172 ms on 4 threads, plus 370 ms to load, so it needs a long-lived worker.
   - It is 565 MB, mostly the 256k-token embedding table.
   - Next to try: INT8 quantization, a vocabulary trimmed to the tokens seen, or a smaller encoder (Ettin-32M) fine-tuned the same way.

6. **`code_simple` belongs at `low`.** Jev sent 110 of the 163 prompts needing `low` to `medium`, because `effort.levels` mapped `code_simple` (commit, run the tests, a rename) to `medium`. Mapping it to `low` lifts Jev from 44% to 54% exact (MAE 0.60 → 0.54) on the same 509 prompts. This is now the default.

**Not measured.**
- Agreement between judges.
- A human-labelled test set.
- The model inside the router.

## 4. Smaller, faster, and combined with Jev

Same 509 held-out prompts (96 sessions). `combine.py` scores combinations; "cost" runs add an expected-cost term to the loss that charges a too-low effort twice as much as a too-high one (`--cost-weight 1 --under-cost 2`). Latency is ONNX Runtime on CPU, median per prompt.

| Approach | Exact | ±1 | Under | Over | MAE | Size, latency (4 threads / 1), load |
|---|---:|---:|---:|---:|---:|---|
| Jev, `code_simple` at `low` | 54% | 93% | 33% | 13% | 0.54 | network |
| mmBERT-small (section 3) | 59% | 95% | 23% | 18% | 0.47 | 565 MB, 172 / 405 ms, 370 ms |
| mmBERT-small, cost | 60% | 95% | 22% | 17% | 0.46 | 565 MB, 160 / 392 ms, 330 ms |
| **Ettin-32M, cost** (embeddings trained, lr 8e-5) | **60%** | 94% | 25% | 15% | 0.47 | **129 MB, 55 / 128 ms, 120 ms** |
| Ettin-32M + Jev, 50/50 expected level | 64% | 98% | 23% | 13% | 0.39 | |
| Ettin-32M or Jev, the higher | 60% | 96% | 15% | 25% | 0.45 | |
| mmBERT-small + Jev, 50/50 expected level | 64% | 99% | 22% | 14% | 0.38 | |
| mmBERT-small, cost + Jev, stacked (CV by session) | 66% | 96% | 20% | 14% | 0.39 | |

**Findings.**

1. **Ettin-32M matches mmBERT-small at a quarter of the size and latency.** 60% exact, 129 MB, 55 ms on 4 threads and a 120 ms load: cheap enough to run on every prompt.
2. **The under-effort penalty barely moves under-effort** (mmBERT 23% → 22%). The labels are likely too noisy for the loss to separate the two directions. Taking the higher of the model and Jev does it instead: under-effort falls to 12–15%, for about 10 more points of over-effort.
3. **The model and Jev disagree usefully.** Averaging their expected effort levels lifts either model from about 60% to 64% exact, and the cases off by two or more levels almost vanish (±1 at 98–99%). A learned combination is no better within noise (63–66%).
4. **INT8 dynamic quantization breaks the model.** With the stored shapes dropped so the quantizer can run, mmBERT-small predicts `high` for nearly everything (9% exact). Ettin-32M makes it unnecessary.

**Recommendation for the router.** Ettin-32M alone as the local classifier, or averaged with Jev where a Jev key is set. Use "the higher of the two" when under-effort costs more than tokens.
