"""Asks Jev for the held-out sessions of the labelled dataset, to compare it
with the fine-tuned model on the same prompts.

    python3 experiments/jev_on_dataset.py
    python3 experiments/jev_on_dataset.py --report

Uses the same session split as train_classifier.py (seed 0, a fifth of the
items). Each prompt goes with its context through the installed
system1router binary, as in production; this sends those past prompts to
TypeSafe. Answers go to <dataset>/jev-test.jsonl (mode 0600); --report
scores them against the judge's labels.
"""
from __future__ import annotations

import argparse
import collections
import json
import os
import random
import subprocess
import tempfile
import time
from pathlib import Path

STATE = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state") / "system1router"
DATASET = STATE / "dataset"
EFFORTS = ["low", "medium", "high", "xhigh", "max"]
CLASS_EFFORT = {"question_simple": "low", "question_moderate": "medium", "question_hard": "high", "code_simple": "medium", "code_moderate": "high", "code_hard": "xhigh"}
TIERS = ("simple", "moderate", "hard")


def labelled() -> list[dict]:
    labels = {}
    for path in sorted((DATASET / "shards").glob("labels-*.jsonl")):
        for line in path.read_text().splitlines():
            if line.strip():
                row = json.loads(line)
                labels[row["id"]] = row
    items = [json.loads(line) for line in (DATASET / "items.jsonl").read_text().splitlines()]
    return [{**item, "label": labels[item["id"]]} for item in items if labels.get(item["id"], {}).get("effort") in EFFORTS]


def test_split(items: list[dict], share: float = 0.2, seed: int = 0) -> list[dict]:
    """train_classifier.split, held-out side."""
    sessions = sorted({item["session"] for item in items})
    random.Random(seed).shuffle(sessions)
    sizes = collections.Counter(item["session"] for item in items)
    held, count = set(), 0
    for session in sessions:
        if count >= share * len(items):
            break
        held.add(session)
        count += sizes[session]
    return [item for item in items if item["session"] in held]


def upper(probabilities: dict, top: str, share: float = 0.2) -> str:
    mass = {kind: [probabilities.get(f"{kind}_{tier}", 0.0) for tier in TIERS] for kind in ("question", "code")}
    kind = "code" if sum(mass["code"]) > sum(mass["question"]) or (sum(mass["code"]) == sum(mass["question"]) and top.startswith("code")) else "question"
    total = sum(mass[kind])
    if total <= 0:
        return top
    return next(f"{kind}_{TIERS[i]}" for i in (2, 1, 0) if i == 0 or sum(mass[kind][i:]) / total >= share)


def scores(truth: list[str], predicted: list[str]) -> str:
    pairs = [(EFFORTS.index(t), EFFORTS.index(p)) for t, p in zip(truth, predicted)]
    n = len(pairs)
    exact, near = sum(t == p for t, p in pairs) / n, sum(abs(t - p) <= 1 for t, p in pairs) / n
    under, over = sum(p < t for t, p in pairs) / n, sum(p > t for t, p in pairs) / n
    return f"n={n} exact {exact:.0%} ±1 {near:.0%} under {under:.0%} over {over:.0%} mae {sum(abs(t - p) for t, p in pairs) / n:.2f}"


def run() -> None:
    test = test_split(labelled())
    with tempfile.TemporaryDirectory() as scratch:
        config = Path(scratch) / "config.json"
        config.write_text(json.dumps({"routing": {"enabled": True, "swap": "effort", "classSelection": "argmax"},
                                      "providers": {"selected": "jev", "jev": {"credentialEnv": "JEV_API_KEY", "model": "jev-1.14.0"}},
                                      "telemetry": {"enabled": False}, "timeouts": {"providerMs": 10000}}))
        rows = []
        for index, item in enumerate(test):
            log = Path(scratch) / f"{index}.jsonl"
            event = {"protocolVersion": "1", "host": "generic", "event": "user_prompt", "sessionId": "dataset", "request": {"query": item["prompt"]},
                     "currentModel": {"provider": "anthropic", "model": "claude-opus-5-5", "variant": "none"}, "context": {"conversationSummary": item["context"]}}
            env = {**os.environ, "SYSTEM1ROUTER_VERBOSE": "1", "SYSTEM1ROUTER_VERBOSE_LOG": str(log), "XDG_STATE_HOME": scratch, "SYSTEM1ROUTER_TELEMETRY_DISABLED": "1"}
            started = time.perf_counter()
            subprocess.run(["system1router", "--config", str(config)], input=json.dumps(event), capture_output=True, text=True, env=env, check=False)
            response = next((e["body"] for e in map(json.loads, log.read_text().splitlines()) if e["direction"] == "response"), None) if log.exists() else None
            answer = (response or {}).get("answers", {}).get("routing_class", {})
            rows.append({"id": item["id"], "choice": answer.get("choice"), "probabilities": answer.get("probabilities"), "ms": round((time.perf_counter() - started) * 1000)})
            if index % 50 == 0:
                print(f"{index}/{len(test)}", flush=True)
    out = DATASET / "jev-test.jsonl"
    out.write_text("".join(json.dumps(row) + "\n" for row in rows))
    os.chmod(out, 0o600)
    print(f"{len(rows)} answers, {sum(1 for r in rows if r['choice'])} with a class -> {out}")


def report() -> None:
    test = {item["id"]: item for item in test_split(labelled())}
    rows = [json.loads(line) for line in (DATASET / "jev-test.jsonl").read_text().splitlines()]
    rows = [row for row in rows if row["choice"] and row["id"] in test]
    truth = [test[row["id"]]["label"]["effort"] for row in rows]
    print("Jev argmax         ", scores(truth, [CLASS_EFFORT[row["choice"]] for row in rows]))
    print("Jev upper 20%      ", scores(truth, [CLASS_EFFORT[upper(row["probabilities"] or {}, row["choice"])] for row in rows]))
    print("judge class mapped ", scores(truth, [CLASS_EFFORT[test[row["id"]]["label"]["class"]] for row in rows]), "(the effort.levels mapping's own ceiling)")
    ms = sorted(row["ms"] for row in rows)
    print(f"latency median {ms[len(ms) // 2]} ms, p95 {ms[int(len(ms) * 0.95)]} ms")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", action="store_true")
    report() if parser.parse_args().report else run()
