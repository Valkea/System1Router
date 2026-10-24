"""Summarises an effort-swap trial from the router's local logs.

    python3 experiments/trial_report.py [--since 2026-10-24] [--export labels.jsonl]

Reads `events.jsonl` and `usage.jsonl` in $XDG_STATE_HOME/system1router (or
~/.local/state/system1router): how many prompts were classified, the
efforts chosen, where the upper-tier selection differed from the median
and from Jev's top choice, and the manual /effort overrides. `--export`
writes the captured prompts with their context and the router's choice, to
label by hand or with a judge model; keep that file out of the repository.
"""
from __future__ import annotations

import argparse
import collections
import datetime
import json
import os
from pathlib import Path

TIERS = ("simple", "moderate", "hard")


def state_directory() -> Path:
    base = os.environ.get("XDG_STATE_HOME")
    return Path(base) / "system1router" if base else Path.home() / ".local/state/system1router"


def rows(path: Path, since_ms: int) -> list[dict]:
    if not path.exists():
        return []
    found = []
    for line in path.read_text().splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        stamp = row.get("timestampMs") or int(row.get("correlationId", "s1r-0-0").split("-")[1], 16)
        if stamp >= since_ms:
            found.append(row)
    return found


def pick(probabilities: dict[str, float], share: float) -> str | None:
    """The kind by mass, then the hardest tier whose upper tail holds `share`."""
    mass = {kind: [probabilities.get(f"{kind}_{tier}", 0.0) for tier in TIERS] for kind in ("question", "code")}
    kind = "code" if sum(mass["code"]) >= sum(mass["question"]) else "question"
    total = sum(mass[kind])
    if total <= 0:
        return None
    index = next(i for i in (2, 1, 0) if i == 0 or sum(mass[kind][i:]) / total >= share)
    return f"{kind}_{TIERS[index]}"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--since", help="YYYY-MM-DD, local time")
    parser.add_argument("--export", type=Path)
    args = parser.parse_args()
    since = int(datetime.datetime.fromisoformat(args.since).timestamp() * 1000) if args.since else 0
    directory = state_directory()
    events = [row for row in rows(directory / "events.jsonl", since) if row.get("host") == "claude-code" and row.get("routingClass")]
    usage = rows(directory / "usage.jsonl", since)
    overrides = [row for row in usage if row.get("kind") == "override"]

    print(f"Log: {directory}")
    print(f"Classified prompts: {len(events)} in {len({row.get('capture', {}).get('sessionId') for row in events})} sessions; Jev models: {dict(collections.Counter(row.get('providerModel') for row in events))}")
    print(f"Outcomes: {dict(collections.Counter(row.get('policyOutcome') for row in events))}")
    print(f"Classes: {dict(collections.Counter(row['routingClass'] for row in events).most_common())}")
    print(f"Efforts applied: {dict(collections.Counter(row.get('effort') for row in events if row.get('effort')).most_common())}")
    with_probabilities = [row for row in events if row.get("probabilities")]
    if with_probabilities:
        differs = collections.Counter()
        for row in with_probabilities:
            top = max(row["probabilities"], key=row["probabilities"].get)
            upper, median = pick(row["probabilities"], 0.2), pick(row["probabilities"], 0.5)
            differs["upper != argmax"] += upper != top
            differs["upper != median"] += upper != median
        print(f"Selection changed the class: {dict(differs)} of {len(with_probabilities)}")
    print(f"Manual /effort overrides: {len(overrides)} ({dict(collections.Counter((row.get('from'), row.get('effort')) for row in overrides))})")
    latencies = sorted(row.get("latencyMs", 0) for row in events)
    if latencies:
        print(f"Decision latency: median {latencies[len(latencies) // 2]} ms, p95 {latencies[int(len(latencies) * 0.95)]} ms")
    if args.export:
        overridden = {row.get("sessionId") for row in overrides}
        with args.export.open("w") as out:
            for row in events:
                capture = row.get("capture") or {}
                out.write(json.dumps({"correlationId": row.get("correlationId"), "query": capture.get("query"), "context": capture.get("context"), "routingClass": row["routingClass"], "effort": row.get("effort"),
                                      "probabilities": row.get("probabilities"), "sessionHadOverride": capture.get("sessionId") in overridden, "label": None}, ensure_ascii=False) + "\n")
        os.chmod(args.export, 0o600)
        print(f"Exported {len(events)} decisions to {args.export} (label the `label` field)")


if __name__ == "__main__":
    main()
