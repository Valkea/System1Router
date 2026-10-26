"""Runs an exported effort model on the held-out sessions and saves its
per-item probabilities, timing each prompt.

    uv run --no-project --index https://download.pytorch.org/whl/cpu --index-strategy unsafe-best-match \\
        --with torch --with "transformers>=4.48" --with numpy --with onnxruntime \\
        python -I experiments/predict.py <model dir> [--onnx model.int8.onnx] [--quantize all|matmul]

--quantize first writes model.int8.onnx: dynamic INT8 weights for the
matrix products (matmul), and also the embedding table (all).
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

import numpy as np
import onnxruntime as ort
from transformers import AutoTokenizer

sys.path.insert(0, str(Path(__file__).resolve().parent))
from train_classifier import EFFORTS, STATE, encode, load, risk_averse, scores, split  # noqa: E402


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("model", type=Path)
    parser.add_argument("--onnx", default="model.onnx")
    parser.add_argument("--quantize", choices=["all", "matmul"], help="write and run model.int8.onnx: INT8 weights for MatMul and Gather (all) or MatMul only")
    parser.add_argument("--max-length", type=int, default=256)
    args = parser.parse_args()
    if args.quantize:
        # The quantizer re-infers shapes and trips on the exporter's stored
        # ones, so they are dropped first; the graph itself is unchanged.
        import onnx
        from onnxruntime.quantization import QuantType, quantize_dynamic
        graph = onnx.load(str(args.model / "model.onnx"))
        del graph.graph.value_info[:]
        onnx.save(graph, str(args.model / "model.clean.onnx"), save_as_external_data=True, location="model.clean.onnx.data")
        quantize_dynamic(str(args.model / "model.clean.onnx"), str(args.model / "model.int8.onnx"), weight_type=QuantType.QInt8, op_types_to_quantize=["MatMul", "Gather"] if args.quantize == "all" else ["MatMul"])
        for path in args.model.glob("model.clean.onnx*"):
            path.unlink()
        args.onnx = "model.int8.onnx"
    _, test = split(load(STATE / "dataset"))
    tokenizer = AutoTokenizer.from_pretrained(args.model)
    options = ort.SessionOptions()
    options.intra_op_num_threads = 4
    started = time.perf_counter()
    session = ort.InferenceSession(str(args.model / args.onnx), options, providers=["CPUExecutionProvider"])
    load_ms = (time.perf_counter() - started) * 1000
    rows, times = [], []
    for item in test:
        ids = np.array([encode(tokenizer, item, args.max_length)], dtype=np.int64)
        begun = time.perf_counter()
        logits = session.run(None, {"input_ids": ids, "attention_mask": np.ones_like(ids)})[0][0]
        times.append((time.perf_counter() - begun) * 1000)
        p = np.exp(logits - logits.max())
        rows.append({"id": item["id"], "probabilities": [round(float(x), 5) for x in p / p.sum()]})
    out = args.model / f"test-predictions{'' if args.onnx == 'model.onnx' else '.' + args.onnx.split('.')[1]}.jsonl"
    out.write_text("".join(json.dumps(row) + "\n" for row in rows))
    os.chmod(out, 0o600)
    truth = [item["label"]["effort"] for item in test]
    probabilities = [np.array(row["probabilities"]) for row in rows]
    size = sum(path.stat().st_size for path in args.model.glob(args.onnx + "*")) / 1e6
    print(json.dumps({"onnx": args.onnx, "megabytes": round(size), "loadMs": round(load_ms), "medianMs": round(float(np.median(times)), 1), "p95Ms": round(float(np.percentile(times, 95)), 1),
                      "argmax": scores(truth, [EFFORTS[int(p.argmax())] for p in probabilities]), "upper 20%": scores(truth, [EFFORTS[risk_averse(p, 0.2)] for p in probabilities])}, indent=1))


if __name__ == "__main__":
    main()
