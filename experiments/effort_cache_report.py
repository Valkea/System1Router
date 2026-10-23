"""Tabulates experiments/effort_cache.sh: per turn, the effort the request
carried (from the session transcript's request log) and how much of the
prompt the cache served."""
import json
import sys
from pathlib import Path


def requests(session: str) -> list[dict]:
    """The main-loop requests Claude Code logged for a session, in order."""
    found = next((Path.home() / ".claude/projects").glob(f"*/{session}.jsonl"), None)
    if found is None:
        return []
    rows = (json.loads(line) for line in found.read_text().splitlines() if line.strip())
    return [row for row in rows if row.get("type") == "api-request" and row.get("querySource") == "sdk"]


def main(out: Path) -> None:
    print(f"{'run':7} {'turn':12} {'model':18} {'effort':6} {'per-turn':>8} {'read':>7} {'write':>7} {'read%':>5}")
    for sid_file in sorted(out.glob("*.sid")):
        run = sid_file.stem
        sent = requests(sid_file.read_text().strip())
        for index, path in enumerate(sorted(out.glob(f"{run}-*.json"))):
            usage = json.loads(path.read_text())["usage"]
            read, write, fresh = usage["cache_read_input_tokens"], usage["cache_creation_input_tokens"], usage["input_tokens"]
            total = read + write + fresh
            params = sent[index]["params"] if index < len(sent) else {}
            effort = (params.get("output_config") or {}).get("effort", "?")
            print(f"{run:7} {path.stem.split('-', 1)[1]:12} {params.get('model', '?'):18} {effort:6} {total:8} {read:7} {write:7} {round(100 * read / total) if total else 0:4}%")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
