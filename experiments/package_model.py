"""Packages a trained effort model as the three files the router downloads.

    uv run --no-project --with onnx --with "transformers>=4.48" \\
        python -I experiments/package_model.py <model dir> <out dir> [--name ettin-32m-effort-v1]

Writes, into <out dir>:
  <name>.onnx            the ONNX graph with its weights embedded (one file)
  <name>.tokenizer.json  the Hugging Face tokenizer
  <name>.json            the manifest: efforts, input layout, file sizes

These are uploaded to the Hugging Face model repository the router
downloads from, or any folder or URL `providers.local.url` points at. The input layout must match
train_classifier.encode, which the router reproduces.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
from pathlib import Path

import onnx
from transformers import AutoTokenizer

EFFORTS = ["low", "medium", "high", "xhigh", "max"]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("model", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--name", default="ettin-32m-effort-v1")
    parser.add_argument("--max-length", type=int, default=256)
    parser.add_argument("--base", default="jhu-clsp/ettin-encoder-32m", help="the encoder it was fine-tuned from")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    graph = onnx.load(str(args.model / "model.onnx"))
    onnx.save(graph, str(args.out / f"{args.name}.onnx"))
    shutil.copy(args.model / "tokenizer.json", args.out / f"{args.name}.tokenizer.json")
    tokenizer = AutoTokenizer.from_pretrained(args.model)
    files = {kind: f"{args.name}.{kind}" for kind in ("onnx", "tokenizer.json")}
    manifest = {"name": args.name, "base": args.base, "efforts": EFFORTS, "maxLength": args.max_length, "contextChars": 4000,
                "cls": tokenizer.cls_token_id, "sep": tokenizer.sep_token_id,
                "files": {kind: {"name": name, "bytes": (args.out / name).stat().st_size, "sha256": hashlib.sha256((args.out / name).read_bytes()).hexdigest()} for kind, name in files.items()}}
    (args.out / f"{args.name}.json").write_text(json.dumps(manifest, indent=1) + "\n")
    print(json.dumps(manifest, indent=1))


if __name__ == "__main__":
    main()
