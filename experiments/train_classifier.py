"""Fine-tunes a small encoder to predict the effort a prompt needs.

    uv run --no-project --index https://download.pytorch.org/whl/cpu --index-strategy unsafe-best-match \\
        --with torch --with "transformers>=4.48" --with numpy --with onnx --with onnxscript --with onnxruntime \\
        python -I experiments/train_classifier.py [--model jhu-clsp/mmBERT-small] [--epochs 4]

Reads the dataset built by dataset_extract.py and the judge's labels
(shards/labels-*.jsonl) from $XDG_STATE_HOME/system1router/dataset, holds out
a fifth of the sessions, fine-tunes on CPU, and reports the held-out
sessions against simple baselines and against Jev where the trial log has
Jev's answer for the same prompt. Writes the model, its ONNX export and
report.json next to the dataset, never into the repository.
"""
from __future__ import annotations

import argparse
import collections
import json
import math
import os
import random
import re
import time
from pathlib import Path

import numpy as np
import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

EFFORTS = ["low", "medium", "high", "xhigh", "max"]
# effort.levels defaults (src/config.rs), to score Jev's class as an effort
CLASS_EFFORT = {"question_simple": "low", "question_moderate": "medium", "question_hard": "high", "code_simple": "medium", "code_moderate": "high", "code_hard": "xhigh"}
STATE = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state") / "system1router"


def load(dataset: Path) -> list[dict]:
    labels = {}
    for path in sorted((dataset / "shards").glob("labels-*.jsonl")):
        for line in path.read_text().splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get("effort") in EFFORTS:
                labels[row["id"]] = row
    items = [json.loads(line) for line in (dataset / "items.jsonl").read_text().splitlines()]
    return [{**item, "label": labels[item["id"]]} for item in items if item["id"] in labels]


def split(items: list[dict], share: float = 0.2, seed: int = 0) -> tuple[list[dict], list[dict]]:
    """Whole sessions go to one side, so no conversation is in both."""
    sessions = sorted({item["session"] for item in items})
    random.Random(seed).shuffle(sessions)
    sizes = collections.Counter(item["session"] for item in items)
    held, count = set(), 0
    for session in sessions:
        if count >= share * len(items):
            break
        held.add(session)
        count += sizes[session]
    return [item for item in items if item["session"] not in held], [item for item in items if item["session"] in held]


def encode(tokenizer, item: dict, max_length: int) -> list[int]:
    """[CLS] request [SEP] newest context [SEP]: the request is kept whole up
    to a third of the window, the context is cut from its oldest end."""
    request = tokenizer(item["prompt"], add_special_tokens=False)["input_ids"][: max_length // 3]
    context = tokenizer(item["context"][-4000:], add_special_tokens=False)["input_ids"]
    room = max_length - len(request) - 3
    context = context[-room:] if room > 0 else []
    return [tokenizer.cls_token_id, *request, tokenizer.sep_token_id, *context, tokenizer.sep_token_id]


def batches(rows: list[list[int]], targets: list[int] | None, size: int, pad: int, shuffle: bool, seed: int):
    order = list(range(len(rows)))
    if shuffle:
        random.Random(seed).shuffle(order)
    for start in range(0, len(order), size):
        chunk = order[start:start + size]
        width = max(len(rows[i]) for i in chunk)
        ids = torch.full((len(chunk), width), pad, dtype=torch.long)
        mask = torch.zeros((len(chunk), width), dtype=torch.long)
        for row, index in enumerate(chunk):
            ids[row, : len(rows[index])] = torch.tensor(rows[index])
            mask[row, : len(rows[index])] = 1
        yield chunk, ids, mask, (torch.tensor([targets[i] for i in chunk]) if targets is not None else None)


def scores(truth: list[str], predicted: list[str | None]) -> dict:
    pairs = [(EFFORTS.index(t), EFFORTS.index(p)) for t, p in zip(truth, predicted) if p in EFFORTS]
    n = len(pairs) or 1
    return {"n": len(pairs), "exact": sum(t == p for t, p in pairs) / n, "within1": sum(abs(t - p) <= 1 for t, p in pairs) / n,
            "under": sum(p < t for t, p in pairs) / n, "over": sum(p > t for t, p in pairs) / n, "mae": sum(abs(t - p) for t, p in pairs) / n}


def risk_averse(probabilities: np.ndarray, share: float) -> int:
    """The highest effort whose upper tail holds `share` of the probability."""
    return next(i for i in range(len(probabilities) - 1, -1, -1) if i == 0 or probabilities[i:].sum() >= share)


def key(text: str) -> str:
    """The start of a request in letters only: the router redacts paths and secrets on its side."""
    return re.sub(r"[^a-z]", "", text.split("/")[0].lower())[:60]


def jev_answers() -> dict[tuple, dict]:
    """Jev's trial decisions by (session, request start): what it said for a prompt."""
    found = {}
    path = STATE / "events.jsonl"
    for line in path.read_text().splitlines() if path.exists() else []:
        try:
            row = json.loads(line)
        except ValueError:
            continue
        capture = row.get("capture") or {}
        if row.get("routingClass") and capture.get("query"):
            found[(capture.get("sessionId"), key(capture["query"]))] = row
    return found


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="jhu-clsp/mmBERT-small")
    parser.add_argument("--epochs", type=int, default=4)
    parser.add_argument("--batch", type=int, default=16)
    parser.add_argument("--lr", type=float, default=5e-5)
    parser.add_argument("--max-length", type=int, default=384)
    parser.add_argument("--dataset", type=Path, default=STATE / "dataset")
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--train-embeddings", action="store_true", help="also train the token embeddings (mmBERT's are 256k x 384: memory heavy)")
    args = parser.parse_args()
    torch.manual_seed(0)
    torch.set_num_threads(args.threads)

    items = load(args.dataset)
    train, test = split(items)
    print(f"{len(items)} labelled items: {len(train)} train, {len(test)} test ({len({i['session'] for i in test})} held-out sessions)", flush=True)
    print("labels:", dict(collections.Counter(i["label"]["effort"] for i in items)), flush=True)

    tokenizer = AutoTokenizer.from_pretrained(args.model)
    model = AutoModelForSequenceClassification.from_pretrained(args.model, num_labels=len(EFFORTS))
    pad = tokenizer.pad_token_id
    if not args.train_embeddings:
        # Frozen embeddings: no optimizer state for the large vocabulary table.
        model.get_input_embeddings().weight.requires_grad_(False)
    x_train = [encode(tokenizer, item, args.max_length) for item in train]
    y_train = [EFFORTS.index(item["label"]["effort"]) for item in train]
    x_test = [encode(tokenizer, item, args.max_length) for item in test]
    counts = collections.Counter(y_train)
    # Inverse square-root frequency: rare efforts count more without dominating.
    weights = torch.tensor([1 / math.sqrt(counts.get(i, 1)) for i in range(len(EFFORTS))], dtype=torch.float)
    weights = weights / weights.mean()
    loss_fn = torch.nn.CrossEntropyLoss(weight=weights)
    optimizer = torch.optim.AdamW([p for p in model.parameters() if p.requires_grad], lr=args.lr, weight_decay=0.01)
    steps = args.epochs * math.ceil(len(x_train) / args.batch)
    scheduler = torch.optim.lr_scheduler.LambdaLR(optimizer, lambda step: min(1.0, step / max(1, steps // 10)) * max(0.0, (steps - step) / max(1, steps - steps // 10)))

    started = time.time()
    step = 0
    for epoch in range(args.epochs):
        model.train()
        total = 0.0
        for _, ids, mask, target in batches(x_train, y_train, args.batch, pad, True, epoch):
            loss = loss_fn(model(input_ids=ids, attention_mask=mask).logits, target)
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()
            scheduler.step()
            optimizer.zero_grad()
            total += loss.item()
            step += 1
            if step % 20 == 0:
                print(f"epoch {epoch + 1} step {step}/{steps} loss {total / (step - epoch * math.ceil(len(x_train) / args.batch)):.3f} {time.time() - started:.0f}s", flush=True)

    model.eval()
    probabilities = np.zeros((len(x_test), len(EFFORTS)))
    with torch.no_grad():
        for chunk, ids, mask, _ in batches(x_test, None, 32, pad, False, 0):
            probabilities[chunk] = torch.softmax(model(input_ids=ids, attention_mask=mask).logits, dim=-1).numpy()
    truth = [item["label"]["effort"] for item in test]
    jev = jev_answers()
    matched = [(i, jev.get((item["session"], key(item["prompt"])))) for i, item in enumerate(test)]
    matched = [(i, row) for i, row in matched if row]
    majority = collections.Counter(y_train).most_common(1)[0][0]
    report = {
        "model": args.model, "train": len(train), "test": len(test), "seconds": round(time.time() - started),
        "majority (" + EFFORTS[majority] + ")": scores(truth, [EFFORTS[majority]] * len(test)),
        "effort the turn ran at": scores(truth, [item["outcome"]["effort"] for item in test]),
        "fine-tuned, argmax": scores(truth, [EFFORTS[int(p.argmax())] for p in probabilities]),
        "fine-tuned, upper 20%": scores(truth, [EFFORTS[risk_averse(p, 0.2)] for p in probabilities]),
        "fine-tuned, upper 30%": scores(truth, [EFFORTS[risk_averse(p, 0.3)] for p in probabilities]),
    }
    if matched:
        subset = [truth[i] for i, _ in matched]
        report["Jev trial decisions (same prompts)"] = scores(subset, [row.get("effort") or CLASS_EFFORT.get(row["routingClass"]) for _, row in matched])
        report["fine-tuned on those prompts, upper 20%"] = scores(subset, [EFFORTS[risk_averse(probabilities[i], 0.2)] for i, _ in matched])
    report["bySource"] = {source: scores([t for t, item in zip(truth, test) if item["source"] == source], [EFFORTS[risk_averse(p, 0.2)] for p, item in zip(probabilities, test) if item["source"] == source]) for source in ("claude-code", "opencode")}

    out = args.dataset / "models" / args.model.split("/")[-1]
    out.mkdir(parents=True, exist_ok=True)
    model.save_pretrained(out)
    tokenizer.save_pretrained(out)
    sample = torch.tensor([x_test[0]])
    torch.onnx.export(model, (sample, torch.ones_like(sample)), out / "model.onnx", input_names=["input_ids", "attention_mask"], output_names=["logits"],
                      dynamic_axes={"input_ids": {0: "batch", 1: "sequence"}, "attention_mask": {0: "batch", 1: "sequence"}, "logits": {0: "batch"}}, opset_version=18)
    import onnxruntime as ort
    for threads in (1, 4):
        options = ort.SessionOptions()
        options.intra_op_num_threads = threads
        begun = time.perf_counter()
        session = ort.InferenceSession(str(out / "model.onnx"), options, providers=["CPUExecutionProvider"])
        load_ms = (time.perf_counter() - begun) * 1000
        times = []
        for row in x_test[:100]:
            ids = np.array([row], dtype=np.int64)
            begun = time.perf_counter()
            session.run(None, {"input_ids": ids, "attention_mask": np.ones_like(ids)})
            times.append((time.perf_counter() - begun) * 1000)
        report[f"onnx {threads} thread(s)"] = {"loadMs": round(load_ms), "medianMs": round(float(np.median(times)), 1), "p95Ms": round(float(np.percentile(times, 95)), 1), "megabytes": round(sum(path.stat().st_size for path in out.glob("model.onnx*")) / 1e6)}
    (out / "report.json").write_text(json.dumps(report, indent=1))
    os.chmod(out / "report.json", 0o600)
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
