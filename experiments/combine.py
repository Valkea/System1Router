"""Combines the fine-tuned model with Jev on the held-out sessions, offline.

    uv run --no-project --with numpy --with scikit-learn python -I experiments/combine.py [model name ...]

Reads each model's test-predictions.jsonl (train_classifier.py or
predict.py), Jev's jev-test.jsonl (jev_on_dataset.py) and the judge's
labels, and scores single classifiers and combinations on the same prompts:
the higher of the two efforts, the model unless Jev is confident the task is
hard, a weighted average of expected effort levels, and a logistic
regression over both outputs (stacked, cross-validated by session so no
session is scored by a combiner that saw it).
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.model_selection import GroupKFold

sys.path.insert(0, str(Path(__file__).resolve().parent))
from jev_on_dataset import CLASS_EFFORT, DATASET, labelled, test_split  # noqa: E402

EFFORTS = ["low", "medium", "high", "xhigh", "max"]
CLASSES = list(CLASS_EFFORT)
# code_simple at low, as effort.levels now defaults (experiments/RESULTS.md)
LEVELS = {**CLASS_EFFORT, "code_simple": "low"}


def scores(truth: np.ndarray, predicted: np.ndarray) -> str:
    n = len(truth)
    return (f"exact {np.mean(truth == predicted):4.0%}  ±1 {np.mean(abs(truth - predicted) <= 1):4.0%}  under {np.mean(predicted < truth):4.0%}  "
            f"over {np.mean(predicted > truth):4.0%}  mae {np.mean(abs(truth - predicted)):.2f}  (n={n})")


def main() -> None:
    names = sys.argv[1:] or ["mmBERT-small"]
    test = {item["id"]: item for item in test_split(labelled())}
    jev = {row["id"]: row for row in map(json.loads, (DATASET / "jev-test.jsonl").read_text().splitlines())}
    ids = [i for i in test if i in jev and jev[i]["choice"]]
    truth = np.array([EFFORTS.index(test[i]["label"]["effort"]) for i in ids])
    groups = np.array([test[i]["session"] for i in ids])
    jev_class = np.array([[(jev[i]["probabilities"] or {}).get(c, float(c == jev[i]["choice"])) for c in CLASSES] for i in ids])
    jev_effort = np.array([EFFORTS.index(LEVELS[jev[i]["choice"]]) for i in ids])
    # Jev's expected effort level under the mapping.
    jev_expected = jev_class @ np.array([EFFORTS.index(LEVELS[c]) for c in CLASSES]) / np.maximum(jev_class.sum(axis=1), 1e-9)
    hard = jev_class[:, [CLASSES.index("code_hard"), CLASSES.index("question_hard")]].sum(axis=1)
    print("Jev, code_simple at low".ljust(52), scores(truth, jev_effort))
    for name in names:
        path = DATASET / "models" / name / "test-predictions.jsonl"
        if not path.exists():
            print(f"{name}: no test-predictions.jsonl")
            continue
        model = {row["id"]: np.array(row["probabilities"]) for row in map(json.loads, path.read_text().splitlines())}
        probabilities = np.array([model[i] for i in ids])
        predicted = probabilities.argmax(axis=1)
        expected = probabilities @ np.arange(len(EFFORTS))
        print(f"{name}".ljust(52), scores(truth, predicted))
        print(f"{name} or Jev, the higher".ljust(52), scores(truth, np.maximum(predicted, jev_effort)))
        for threshold in (0.5, 0.7):
            pick = np.where(hard >= threshold, np.maximum(predicted, jev_effort), predicted)
            print(f"{name}, Jev's effort when it says hard ≥ {threshold}".ljust(52), scores(truth, pick))
        for weight in (0.5, 0.7):
            print(f"{name} {weight:.0%} + Jev {1 - weight:.0%}, expected level".ljust(52), scores(truth, np.clip(np.rint(weight * expected + (1 - weight) * jev_expected), 0, 4).astype(int)))
        features = np.hstack([np.log(probabilities + 1e-6), jev_class])
        stacked = np.zeros_like(truth)
        for train, held in GroupKFold(5).split(features, truth, groups):
            stacked[held] = LogisticRegression(C=1.0, max_iter=2000).fit(features[train], truth[train]).predict(features[held])
        print(f"{name} + Jev, stacked (CV by session)".ljust(52), scores(truth, stacked))


if __name__ == "__main__":
    main()
