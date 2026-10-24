| Classifier | class | code/question | tier MAE | effort | under | over | latency |
|---|---:|---:|---:|---:|---:|---:|---|
| always `code_moderate` (high) | 17% | 50% | 0.67 | 33% | 17% | 50% | 0 |
| Jev jev-1.14.0 (argmax) | 82% | 100% | 0.18 | 82% | 17% | 1% | 247 ms median, 328 ms p95 (process + HTTPS) |
| Jev jev-1.14.0 (median tier) | 81% | 100% | 0.19 | 81% | 18% | 1% | 247 ms median, 328 ms p95 (process + HTTPS) |
| Jev jev-1.14.0 (upper tier, 20%) | 94% | 100% | 0.06 | 94% | 3% | 3% | 247 ms median, 328 ms p95 (process + HTTPS) |
| Jev, follow-up instructions (argmax) | 82% | 100% | 0.18 | 82% | 17% | 1% |  |
| Jev, follow-up instructions (upper tier, 20%) | 95% | 100% | 0.05 | 95% | 3% | 1% |  |
| minilm-l6 + logistic head, trained on labels | 62% | 87% | 0.28 | 63% | 19% | 18% | 49.0 ms (1 thread), 18.8 ms (4); load 235 ms; 90 MB |
| minilm-l6 + logistic head, trained on Jev labels | 51% | 87% | 0.42 | 53% | 38% | 9% | 49.0 ms (1 thread), 18.8 ms (4); load 235 ms; 90 MB |
| bge-small + logistic head, trained on labels | 61% | 90% | 0.31 | 62% | 20% | 18% | 19.3 ms (1 thread), 9.8 ms (4); load 373 ms; 133 MB |
| bge-small + logistic head, trained on Jev labels | 52% | 85% | 0.39 | 54% | 34% | 12% | 19.3 ms (1 thread), 9.8 ms (4); load 373 ms; 133 MB |
| arctic-xs + logistic head, trained on labels | 59% | 83% | 0.27 | 60% | 21% | 19% | 9.5 ms (1 thread), 4.8 ms (4); load 210 ms; 90 MB |
| arctic-xs + logistic head, trained on Jev labels | 51% | 85% | 0.40 | 53% | 37% | 10% | 9.5 ms (1 thread), 4.8 ms (4); load 210 ms; 90 MB |
| modernbert-embed + logistic head, trained on labels | 68% | 94% | 0.30 | 69% | 14% | 16% | 98.3 ms (1 thread), 55.3 ms (4); load 1451 ms; 596 MB |
| modernbert-embed + logistic head, trained on Jev labels | 58% | 92% | 0.38 | 59% | 36% | 5% | 98.3 ms (1 thread), 55.3 ms (4); load 1451 ms; 596 MB |

Follow-up pairs (same prompt, two conversations; pairs = both of a pair right):

| Classifier | class | pairs | code/question | effort | under | over |
|---|---:|---:|---:|---:|---:|---:|
| Jev, prompt only (argmax) | 38% | 0% | 75% | 38% | 57% | 5% |
| Jev, prompt only (upper tier, 20%) | 18% | 0% | 75% | 25% | 50% | 25% |
| Jev, prompt + conversation (argmax) | 62% | 30% | 95% | 62% | 35% | 2% |
| Jev, prompt + conversation (upper tier, 20%) | 72% | 45% | 98% | 75% | 20% | 5% |
| Jev, prompt + conversation, follow-up instructions (argmax) | 60% | 25% | 95% | 60% | 38% | 2% |
| Jev, prompt + conversation, follow-up instructions (upper tier, 20%) | 75% | 55% | 95% | 78% | 18% | 5% |
| minilm-l6 + head, prompt only | 28% | 0% | 60% | 28% | 52% | 20% |
| minilm-l6 + head, joint text | 68% | 45% | 85% | 68% | 10% | 22% |
| minilm-l6 + head, [prompt ; conversation] | 60% | 35% | 78% | 60% | 28% | 12% |
| bge-small + head, prompt only | 35% | 0% | 85% | 35% | 40% | 25% |
| bge-small + head, joint text | 72% | 55% | 88% | 72% | 18% | 10% |
| bge-small + head, [prompt ; conversation] | 48% | 20% | 82% | 48% | 28% | 25% |
| arctic-xs + head, prompt only | 32% | 0% | 80% | 32% | 38% | 30% |
| arctic-xs + head, joint text | 68% | 50% | 85% | 68% | 18% | 15% |
| arctic-xs + head, [prompt ; conversation] | 55% | 25% | 88% | 55% | 22% | 22% |
| modernbert-embed + head, prompt only | 40% | 0% | 90% | 42% | 42% | 15% |
| modernbert-embed + head, joint text | 78% | 55% | 92% | 78% | 12% | 10% |
| modernbert-embed + head, [prompt ; conversation] | 52% | 25% | 88% | 52% | 22% | 25% |
