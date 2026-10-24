"""Effort classifier bench: Jev against small local encoders.

    python3 experiments/classifier_bench.py jev <out> [base|followups|followups-blind] [questions.json]
        Classifies experiments/prompts.jsonl (base) or the follow-up pairs of
        experiments/followups.jsonl, with or without their conversation, with
        Jev through the system1router binary (key from
        ~/.config/system1router/.env, never read here) and records class,
        probabilities and latency. A questions file replaces Jev's default
        questions (`providers.jev.questions`); its results are named `-custom`.
    uv run --no-project --with onnxruntime --with tokenizers --with huggingface_hub --with numpy \\
        python -I experiments/classifier_bench.py embed <out> <model>
        Embeds the prompts with one ONNX encoder on CPU and times it.
    uv run --no-project --with numpy --with scikit-learn \\
        python -I experiments/classifier_bench.py report <out>
        Scores Jev and a logistic head on each encoder (repeated 5-fold CV)
        against the labels, as classes and as the efforts they map to.

The prompts are synthetic and labelled by hand by their author, so scores
measure agreement with one labeller on one style of prompt, not ground truth.
The follow-ups come in pairs: the same short prompt ("ok do it", "why?")
after two conversations that make it simple or hard, so a classifier that
ignores the conversation gets at most one of each pair right.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CLASSES = ["question_simple", "question_moderate", "question_hard", "code_simple", "code_moderate", "code_hard"]
# effort.levels defaults (src/config.rs)
EFFORT = {"question_simple": "low", "question_moderate": "medium", "question_hard": "high", "code_simple": "medium", "code_moderate": "high", "code_hard": "xhigh"}
LADDER = ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
MODELS = {
    # name: (repo, pooling, prefix)
    "minilm-l6": ("sentence-transformers/all-MiniLM-L6-v2", "mean", ""),
    "bge-small": ("BAAI/bge-small-en-v1.5", "cls", ""),
    "arctic-xs": ("Snowflake/snowflake-arctic-embed-xs", "cls", ""),
    "granite-r2-small": ("ibm-granite/granite-embedding-small-english-r2", "cls", ""),
    "modernbert-embed": ("nomic-ai/modernbert-embed-base", "mean", "classification: "),
}


def prompts(name: str = "prompts") -> list[dict]:
    return [json.loads(line) for line in (HERE / f"{name}.jsonl").read_text().splitlines() if line.strip()]


def rendered(item: dict) -> str:
    """The conversation as the router renders it for Jev (render_context)."""
    parts = []
    if item.get("files"):
        parts.append("Files in play: " + ", ".join(item["files"]))
    if item.get("tools"):
        parts.append("Tools used: " + ", ".join(item["tools"]))
    if item.get("conversation"):
        parts.append("Recent conversation:\n" + item["conversation"])
    return "\n".join(parts)


def median_class(probabilities: dict[str, float], top: str) -> str:
    """The router's `select_class`: kind by mass, then the median tier."""
    mass = {kind: [0.0, 0.0, 0.0] for kind in ("question", "code")}
    for name, value in probabilities.items():
        if name in CLASSES and value > 0:
            kind, tier = name.split("_")
            mass[kind][["simple", "moderate", "hard"].index(tier)] += value
    totals = {kind: sum(values) for kind, values in mass.items()}
    if totals["question"] + totals["code"] <= 0:
        return top
    kind = "code" if totals["code"] > totals["question"] or (totals["code"] == totals["question"] and top.startswith("code")) else "question"
    cumulative = 0.0
    for index, value in enumerate(mass[kind]):
        cumulative += value
        if cumulative >= totals[kind] / 2:
            return f"{kind}_{['simple', 'moderate', 'hard'][index]}"
    return f"{kind}_hard"


def run_jev(out: Path, which: str, questions: Path | None = None) -> None:
    out.mkdir(parents=True, exist_ok=True)
    items = prompts() if which == "base" else prompts("followups")
    binary = REPO / "target/debug/system1router"
    with tempfile.TemporaryDirectory() as scratch:
        config = Path(scratch) / "config.json"
        config.write_text(json.dumps({
            "routing": {"enabled": True, "swap": "effort"},
            "providers": {"selected": "jev", "jev": {"credentialEnv": "JEV_API_KEY", **({"questions": json.loads(questions.read_text())} if questions else {})}},
            "telemetry": {"enabled": False},
            "timeouts": {"providerMs": 10000},
        }))
        rows = []
        for index, item in enumerate(items):
            log = Path(scratch) / f"{index}.jsonl"
            event = {"protocolVersion": "1", "host": "generic", "event": "user_prompt", "sessionId": "bench", "request": {"query": item["text"]},
                     "currentModel": {"provider": "anthropic", "model": "claude-opus-5-5", "variant": "none"}}
            if which == "followups":
                event["context"] = {"conversationSummary": item["conversation"], "files": item["files"], "tools": item["tools"]}
            env = {**os.environ, "SYSTEM1ROUTER_VERBOSE": "1", "SYSTEM1ROUTER_VERBOSE_LOG": str(log), "XDG_STATE_HOME": scratch, "SYSTEM1ROUTER_TELEMETRY_DISABLED": "1"}
            started = time.perf_counter()
            done = subprocess.run([str(binary), "--config", str(config)], input=json.dumps(event), capture_output=True, text=True, env=env, check=False)
            elapsed = (time.perf_counter() - started) * 1000
            response = next((entry["body"] for entry in map(json.loads, log.read_text().splitlines()) if entry["direction"] == "response"), None) if log.exists() else None
            answer = (response or {}).get("answers", {}).get("routing_class", {})
            decision = json.loads(done.stdout or "{}")
            rows.append({"label": item["label"], "pair": item.get("pair"), "choice": answer.get("choice"), "probabilities": answer.get("probabilities"), "confidence": answer.get("confidence"),
                         "model": (response or {}).get("model"), "ms": round(elapsed), "outcome": decision.get("outcome"), "code": decision.get("diagnostic", {}).get("code")})
            print(f"{index:3} {item['label']:18} -> {answer.get('choice')!s:18} {elapsed:6.0f} ms", flush=True)
    suffix = "-custom" if questions else ""
    (out / ("jev" + ("" if which == "base" else f"-{which}") + f"{suffix}.jsonl")).write_text("".join(json.dumps(row) + "\n" for row in rows))


def run_embed(out: Path, name: str) -> None:
    import numpy as np
    import onnxruntime as ort
    from huggingface_hub import hf_hub_download, list_repo_files
    from tokenizers import Tokenizer

    repo, pooling, prefix = MODELS[name]
    cache = out / "hf"
    files = list_repo_files(repo)
    onnx = next((f for f in ("onnx/model.onnx", "model.onnx", "onnx/model_quantized.onnx") if f in files), None)
    if onnx is None:
        raise SystemExit(f"{repo}: no ONNX export among {len(files)} files")
    for extra in (f for f in files if f.startswith(onnx) and f != onnx):  # external weights (model.onnx_data)
        hf_hub_download(repo, extra, cache_dir=cache)
    model_path = hf_hub_download(repo, onnx, cache_dir=cache)
    tokenizer = Tokenizer.from_file(hf_hub_download(repo, "tokenizer.json", cache_dir=cache))
    tokenizer.enable_truncation(256)
    texts = [prefix + item["text"] for item in prompts()]
    result = {"model": name, "repo": repo, "onnx": onnx, "megabytes": round(Path(model_path).stat().st_size / 1e6)}
    followups = prompts("followups")
    # The request first, then the newest 1,500 characters of the conversation,
    # so truncation never drops the request or the latest turn.
    variants = {
        "followups-prompt": [prefix + item["text"] for item in followups],
        "followups-context": [prefix + rendered(item)[-1500:] for item in followups],
        "followups-joint": [prefix + f"Current request: {item['text']}\n\n{rendered(item)[-1500:]}" for item in followups],
    }
    for threads in (1, 4):
        options = ort.SessionOptions()
        options.intra_op_num_threads = threads
        started = time.perf_counter()
        session = ort.InferenceSession(model_path, options, providers=["CPUExecutionProvider"])
        result[f"loadMs{threads}"] = round((time.perf_counter() - started) * 1000)
        inputs = {i.name for i in session.get_inputs()}
        vectors, times = [], []
        for text in texts:
            started = time.perf_counter()
            encoding = tokenizer.encode(text)
            ids = np.array([encoding.ids], dtype=np.int64)
            feed = {"input_ids": ids, "attention_mask": np.ones_like(ids)}
            if "token_type_ids" in inputs:
                feed["token_type_ids"] = np.zeros_like(ids)
            hidden = session.run(None, {k: v for k, v in feed.items() if k in inputs})[0][0]
            vector = hidden if hidden.ndim == 1 else (hidden[0] if pooling == "cls" else hidden.mean(axis=0))
            vectors.append(vector / np.linalg.norm(vector))
            times.append((time.perf_counter() - started) * 1000)
        result[f"medianMs{threads}"] = round(float(np.median(times)), 1)
        result[f"p95Ms{threads}"] = round(float(np.percentile(times, 95)), 1)
    np.save(out / f"{name}.npy", np.stack(vectors))
    tokenizer.enable_truncation(512)
    for variant, batch in variants.items():
        encoded = []
        for text in batch:
            encoding = tokenizer.encode(text)
            ids = np.array([encoding.ids], dtype=np.int64)
            feed = {"input_ids": ids, "attention_mask": np.ones_like(ids), "token_type_ids": np.zeros_like(ids)}
            hidden = session.run(None, {k: v for k, v in feed.items() if k in inputs})[0][0]
            vector = hidden if hidden.ndim == 1 else (hidden[0] if pooling == "cls" else hidden.mean(axis=0))
            encoded.append(vector / np.linalg.norm(vector))
        np.save(out / f"{name}-{variant}.npy", np.stack(encoded))
    (out / f"{name}.json").write_text(json.dumps(result))
    print(json.dumps(result))


def upper_class(probabilities: dict[str, float], top: str, share: float = 0.2) -> str:
    """Risk-averse tier: the hardest tier whose upper tail holds `share` of
    the kind's probability mass (median is share 0.5)."""
    mass = {kind: [probabilities.get(f"{kind}_{tier}", 0.0) for tier in ("simple", "moderate", "hard")] for kind in ("question", "code")}
    kind = "code" if sum(mass["code"]) > sum(mass["question"]) or (sum(mass["code"]) == sum(mass["question"]) and top.startswith("code")) else "question"
    total = sum(mass[kind])
    if total <= 0:
        return top
    return next(f"{kind}_{tier}" for index, tier in reversed(list(enumerate(("simple", "moderate", "hard")))) if index == 0 or sum(mass[kind][index:]) / total >= share)


def scores(truth: list[str], predicted: list[str]) -> dict:
    tier = lambda name: ["simple", "moderate", "hard"].index(name.split("_")[1])
    effort = lambda name: LADDER.index(EFFORT[name])
    n = len(truth)
    return {
        "class": sum(t == p for t, p in zip(truth, predicted)) / n,
        "kind": sum(t.split("_")[0] == p.split("_")[0] for t, p in zip(truth, predicted)) / n,
        "tierMae": sum(abs(tier(t) - tier(p)) for t, p in zip(truth, predicted)) / n,
        "effort": sum(EFFORT[t] == EFFORT[p] for t, p in zip(truth, predicted)) / n,
        "under": sum(effort(p) < effort(t) for t, p in zip(truth, predicted)) / n,
        "over": sum(effort(p) > effort(t) for t, p in zip(truth, predicted)) / n,
    }


def run_report(out: Path) -> None:
    import numpy as np
    from sklearn.linear_model import LogisticRegression
    from sklearn.model_selection import StratifiedKFold

    truth = [item["label"] for item in prompts()]
    lines = [f"| Classifier | class | code/question | tier MAE | effort | under | over | latency |", "|---|---:|---:|---:|---:|---:|---:|---|"]
    row = lambda name, s, latency: f"| {name} | {s['class']:.0%} | {s['kind']:.0%} | {s['tierMae']:.2f} | {s['effort']:.0%} | {s['under']:.0%} | {s['over']:.0%} | {latency} |"
    lines.append(row("always `code_moderate` (high)", scores(truth, ["code_moderate"] * len(truth)), "0"))
    jev_labels = None
    if (out / "jev.jsonl").exists():
        jev = [json.loads(line) for line in (out / "jev.jsonl").read_text().splitlines()]
        argmax = [row_["choice"] or "code_moderate" for row_ in jev]
        median = [median_class(row_["probabilities"] or {}, row_["choice"] or "code_moderate") for row_ in jev]
        ms = sorted(row_["ms"] for row_ in jev)
        latency = f"{ms[len(ms) // 2]} ms median, {ms[int(len(ms) * 0.95)]} ms p95 (process + HTTPS)"
        lines.append(row(f"Jev {jev[0]['model']} (argmax)", scores(truth, argmax), latency))
        lines.append(row(f"Jev {jev[0]['model']} (median tier)", scores(truth, median), latency))
        lines.append(row(f"Jev {jev[0]['model']} (upper tier, 20%)", scores(truth, [upper_class(r["probabilities"] or {}, r["choice"] or "code_moderate") for r in jev]), latency))
        jev_labels = argmax
    if (out / "jev-custom.jsonl").exists():
        custom = [json.loads(line) for line in (out / "jev-custom.jsonl").read_text().splitlines()]
        lines.append(row("Jev, follow-up instructions (argmax)", scores(truth, [r["choice"] or "code_moderate" for r in custom]), ""))
        lines.append(row("Jev, follow-up instructions (upper tier, 20%)", scores(truth, [upper_class(r["probabilities"] or {}, r["choice"] or "code_moderate") for r in custom]), ""))
    y = np.array(truth)
    for name in MODELS:
        if not (out / f"{name}.npy").exists():
            continue
        x = np.load(out / f"{name}.npy")
        meta = json.loads((out / f"{name}.json").read_text())
        latency = f"{meta['medianMs1']} ms (1 thread), {meta['medianMs4']} ms (4); load {meta['loadMs1']} ms; {meta['megabytes']} MB"
        for teacher, labels in (("labels", y), ("Jev labels", np.array(jev_labels) if jev_labels else None)):
            if labels is None:
                continue
            predicted = np.empty(len(y), dtype=object)
            totals = []
            for seed in range(3):
                for train, test in StratifiedKFold(5, shuffle=True, random_state=seed).split(x, y):
                    head = LogisticRegression(C=4.0, max_iter=5000).fit(x[train], labels[train])
                    predicted[test] = head.predict(x[test])
                totals.append(scores(truth, list(predicted)))
            mean = {key: sum(t[key] for t in totals) / len(totals) for key in totals[0]}
            lines.append(row(f"{name} + logistic head, trained on {teacher}", mean, latency))
    lines += ["", "Follow-up pairs (same prompt, two conversations; pairs = both of a pair right):", "",
              "| Classifier | class | pairs | code/question | effort | under | over |", "|---|---:|---:|---:|---:|---:|---:|"]
    followups = prompts("followups")
    ftruth = [item["label"] for item in followups]
    groups = np.array([item["pair"] for item in followups])

    def frow(name: str, predicted: list[str]) -> str:
        s = scores(ftruth, predicted)
        pairs = sum(all(ftruth[i] == predicted[i] for i in np.where(groups == g)[0]) for g in set(groups)) / len(set(groups))
        return f"| {name} | {s['class']:.0%} | {pairs:.0%} | {s['kind']:.0%} | {s['effort']:.0%} | {s['under']:.0%} | {s['over']:.0%} |"

    for which, title in (("followups-blind", "Jev, prompt only"), ("followups", "Jev, prompt + conversation"), ("followups-custom", "Jev, prompt + conversation, follow-up instructions")):
        if (out / f"jev-{which}.jsonl").exists():
            rows_ = [json.loads(line) for line in (out / f"jev-{which}.jsonl").read_text().splitlines()]
            lines.append(frow(title + " (argmax)", [r["choice"] or "code_moderate" for r in rows_]))
            lines.append(frow(title + " (upper tier, 20%)", [upper_class(r["probabilities"] or {}, r["choice"] or "code_moderate") for r in rows_]))
    for name in MODELS:
        if not (out / f"{name}-followups-joint.npy").exists():
            continue
        base = np.load(out / f"{name}.npy")
        loaded = {variant: np.load(out / f"{name}-followups-{variant}.npy") for variant in ("prompt", "context", "joint")}
        features = {
            "prompt only": (base, loaded["prompt"]),
            "joint text": (base, loaded["joint"]),
            "[prompt ; conversation]": (np.hstack([base, np.zeros_like(base)]), np.hstack([loaded["prompt"], loaded["context"]])),
        }
        fy = np.array(ftruth)
        for title, (bx, fx) in features.items():
            predicted = np.empty(len(fy), dtype=object)
            # Each fold holds out whole pairs; the head also learns from all base prompts.
            for held in np.array_split(np.random.default_rng(0).permutation(sorted(set(groups))), 5):
                test = np.isin(groups, held)
                head = LogisticRegression(C=4.0, max_iter=5000).fit(np.vstack([bx, fx[~test]]), np.concatenate([y, fy[~test]]))
                predicted[test] = head.predict(fx[test])
            lines.append(frow(f"{name} + head, {title}", list(predicted)))
    report = "\n".join(lines)
    (out / "report.md").write_text(report + "\n")
    print(report)


if __name__ == "__main__":
    command, out = sys.argv[1], Path(sys.argv[2])
    {"jev": lambda: run_jev(out, sys.argv[3] if len(sys.argv) > 3 else "base", Path(sys.argv[4]) if len(sys.argv) > 4 else None), "embed": lambda: run_embed(out, sys.argv[3]), "report": lambda: run_report(out)}[command]()
