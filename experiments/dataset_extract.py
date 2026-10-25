"""Builds the effort-classifier dataset from past Claude Code and OpenCode sessions.

    python3 experiments/dataset_extract.py [--out DIR] [--shard-size 100]

For every prompt the person typed it keeps the prompt, the conversation
before it (rendered as the router renders it for its classifier), and what
the turn then took: tool calls, files edited, output and reasoning tokens,
duration, the effort it ran at, whether it was interrupted, how it ended
and the next prompt. Those outcomes let a judge label the effort the prompt
needed in hindsight.

Sources: ~/.claude/projects/*/*.jsonl and ~/.local/share/opencode/opencode.db
(read only). Headless runs (`claude -p`) and scratch projects are skipped.
Secrets are redacted. Everything is written to DIR (default
$XDG_STATE_HOME/system1router/dataset, mode 0600): it holds real prompts and
code, so it never belongs in the repository.
"""
from __future__ import annotations

import argparse
import collections
import datetime
import json
import os
import re
import sqlite3
from pathlib import Path

SECRETS = [
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----", re.S),
    re.compile(r"\b(?:sk|pk|rk)-[A-Za-z0-9_-]{16,}"),
    re.compile(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}|\bgithub_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"\bAKIA[0-9A-Z]{16}\b|\bAIza[0-9A-Za-z_-]{35}\b|\bxox[abprs]-[A-Za-z0-9-]{10,}"),
    re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
    re.compile(r"(?i)\b(password|passwd|secret|token|api[_-]?key|credential)s?\b(\s*[:=]\s*)(\"[^\"]{4,}\"|'[^']{4,}'|[^\s,;]{6,})"),
]
EDIT_TOOLS = {"Edit", "Write", "MultiEdit", "NotebookEdit", "edit", "write", "patch", "multiedit"}
CONTEXT_CHARS = 3000


def redact(text: str) -> str:
    for pattern in SECRETS:
        text = pattern.sub(lambda m: (m.group(1) + m.group(2) + "[REDACTED]") if m.re.groups == 3 else "[REDACTED]", text)
    return text


def clip(text: str, limit: int) -> str:
    text = text.strip()
    return text if len(text) <= limit else text[:limit] + "…"


def file_of(tool_input: dict) -> str | None:
    for key in ("file_path", "path", "notebook_path", "filePath"):
        if isinstance(tool_input.get(key), str):
            return tool_input[key]
    return None


class Turn:
    """What one typed prompt led to, up to the next typed prompt."""

    def __init__(self) -> None:
        self.tools: collections.Counter[str] = collections.Counter()
        self.files: set[str] = set()
        self.output = self.reasoning = 0
        self.end: float | None = None
        self.efforts: collections.Counter[str] = collections.Counter()
        self.models: collections.Counter[str] = collections.Counter()
        self.interrupted = False
        self.final = ""

    def outcome(self, start: float) -> dict:
        return {
            "toolCalls": sum(self.tools.values()), "tools": dict(self.tools.most_common()), "filesEdited": len(self.files),
            "outputTokens": self.output, "reasoningTokens": self.reasoning, "seconds": round(self.end - start) if self.end else 0,
            "effort": self.efforts.most_common(1)[0][0] if self.efforts else None, "model": self.models.most_common(1)[0][0] if self.models else None,
            "interrupted": self.interrupted, "final": clip(redact(self.final), 400),
        }


def stamp(value: str | None) -> float:
    return datetime.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() if value else 0.0


def claude_items(root: Path) -> list[dict]:
    items = []
    for path in sorted(root.glob("*/*.jsonl")):
        if path.parent.name.startswith("-tmp-"):
            continue
        project = path.parent.name.split("-Projets-persos-", 1)[-1]
        history: list[str] = []
        files: list[str] = []
        tools_seen: list[str] = []
        current: dict | None = None
        turn = Turn()
        for line in path.read_text(errors="replace").splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get("isSidechain") or row.get("isMeta") or row.get("type") not in ("user", "assistant"):
                continue
            message = row.get("message") or {}
            content = message.get("content")
            if row["type"] == "user":
                texts = [content] if isinstance(content, str) else [b.get("text", "") for b in content or [] if b.get("type") == "text"]
                text = "\n".join(t for t in texts if t and not t.startswith("<")).strip()
                if any("Request interrupted" in t for t in texts if t):
                    turn.interrupted = True
                    continue
                if isinstance(content, list) and any(b.get("type") == "tool_result" for b in content):
                    for block in content:
                        if block.get("type") == "tool_result":
                            out = block.get("content")
                            out = out if isinstance(out, str) else "\n".join(i.get("text", "") for i in out or [] if isinstance(i, dict))
                            history.append(f"Tool result:\n{clip(out, 500)}")
                    continue
                typed = row.get("promptSource") in ("typed", "queued", "suggestion_accepted") or (row.get("promptSource") is None and row.get("entrypoint") == "cli") or (row.get("promptSource") == "sdk" and row.get("entrypoint") == "claude-desktop")
                if not text or not typed:
                    continue
                if current:
                    current["outcome"] = turn.outcome(current["_start"])
                    current["outcome"]["nextPrompt"] = clip(redact(text), 160)
                    items.append(current)
                current = {"id": f"cc:{row.get('sessionId')}:{row.get('uuid')}", "source": "claude-code", "project": project, "session": row.get("sessionId"), "timestamp": row.get("timestamp"),
                           "prompt": clip(redact(text), 2000), "context": render(history, files, tools_seen), "_start": stamp(row.get("timestamp"))}
                turn = Turn()
                history.append(f"User:\n{clip(text, 900)}")
            else:
                if current is None:
                    continue
                usage = message.get("usage") or {}
                turn.output += usage.get("output_tokens", 0) or 0
                turn.reasoning += ((usage.get("output_tokens_details") or {}).get("thinking_tokens") or 0)
                turn.end = stamp(row.get("timestamp")) or turn.end
                if row.get("effort"):
                    turn.efforts[str(row["effort"])] += 1
                if message.get("model", "").startswith("claude"):
                    turn.models[message["model"]] += 1
                parts = []
                for block in content or []:
                    if block.get("type") == "text" and block.get("text", "").strip():
                        parts.append(clip(block["text"], 900))
                        turn.final = block["text"]
                    elif block.get("type") == "tool_use":
                        name = block.get("name", "tool")
                        turn.tools[name] += 1
                        tools_seen.append(name)
                        target = file_of(block.get("input") or {})
                        if target:
                            files.append(target)
                            if name in EDIT_TOOLS:
                                turn.files.add(target)
                        parts.append(f"Tool {name}: {clip(json.dumps(block.get('input'), ensure_ascii=False), 180)}")
                if parts:
                    history.append("Assistant:\n" + "\n".join(parts))
        if current:
            current["outcome"] = turn.outcome(current["_start"])
            current["outcome"]["nextPrompt"] = None
            items.append(current)
    return items


def render(history: list[str], files: list[str], tools: list[str]) -> str:
    """The router's bounded context: files, tools, then the newest conversation."""
    recent = lambda values, n: list(dict.fromkeys(reversed(values)))[:n]
    parts = []
    if files:
        parts.append("Files in play: " + ", ".join(recent(files, 8)))
    if tools:
        parts.append("Tools used: " + ", ".join(recent(tools, 10)))
    conversation = "\n\n".join(history)[-CONTEXT_CHARS:]
    if conversation:
        parts.append("Recent conversation:\n" + conversation)
    return redact("\n".join(parts))


def opencode_items(db: Path) -> list[dict]:
    if not db.exists():
        return []
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    sessions = {sid: (directory, parent) for sid, directory, parent in con.execute("select id, directory, parent_id from session")}
    parts: dict[str, list[dict]] = collections.defaultdict(list)
    for message_id, data in con.execute("select message_id, data from part order by time_created"):
        parts[message_id].append(json.loads(data))
    by_session: dict[str, list[tuple]] = collections.defaultdict(list)
    for mid, sid, created, data in con.execute("select id, session_id, time_created, data from message order by time_created"):
        by_session[sid].append((mid, created, json.loads(data)))
    items = []
    for sid, messages in by_session.items():
        directory, parent = sessions.get(sid, ("", None))
        if parent or "/tmp/" in (directory or ""):
            continue  # subagent sessions and scratch projects
        history: list[str] = []
        files: list[str] = []
        tools_seen: list[str] = []
        current: dict | None = None
        turn = Turn()
        for mid, created, data in messages:
            if data.get("role") == "user":
                text = "\n".join(p.get("text", "") for p in parts[mid] if p.get("type") == "text" and not p.get("synthetic")).strip()
                if not text or text.startswith("/"):
                    continue
                if current:
                    current["outcome"] = turn.outcome(current["_start"])
                    current["outcome"]["nextPrompt"] = clip(redact(text), 160)
                    items.append(current)
                current = {"id": f"oc:{sid}:{mid}", "source": "opencode", "project": Path(directory or "?").name, "session": sid,
                           "timestamp": datetime.datetime.fromtimestamp(created / 1000, datetime.timezone.utc).isoformat(), "prompt": clip(redact(text), 2000),
                           "context": render(history, files, tools_seen), "_start": created / 1000}
                turn = Turn()
                history.append(f"User:\n{clip(text, 900)}")
            elif current is not None:
                tokens = data.get("tokens") or {}
                turn.output += tokens.get("output", 0) or 0
                turn.reasoning += tokens.get("reasoning", 0) or 0
                completed = (data.get("time") or {}).get("completed")
                turn.end = completed / 1000 if completed else turn.end
                if data.get("modelID"):
                    turn.models[f"{data.get('providerID')}/{data['modelID']}"] += 1
                if data.get("variant"):
                    turn.efforts[str(data["variant"])] += 1
                if (data.get("error") or {}).get("name") == "MessageAbortedError":
                    turn.interrupted = True
                lines = []
                for part in parts[mid]:
                    if part.get("type") == "text" and part.get("text", "").strip() and not part.get("synthetic"):
                        lines.append(clip(part["text"], 900))
                        turn.final = part["text"]
                    elif part.get("type") == "tool":
                        name = part.get("tool", "tool")
                        state = part.get("state") or {}
                        turn.tools[name] += 1
                        tools_seen.append(name)
                        target = file_of(state.get("input") or {})
                        if target:
                            files.append(target)
                            if name in EDIT_TOOLS:
                                turn.files.add(target)
                        lines.append(f"Tool {name}: {clip(json.dumps(state.get('input'), ensure_ascii=False), 180)}")
                        if isinstance(state.get("output"), str):
                            history.append(f"Tool result:\n{clip(state['output'], 500)}")
                if lines:
                    history.append("Assistant:\n" + "\n".join(lines))
        if current:
            current["outcome"] = turn.outcome(current["_start"])
            current["outcome"]["nextPrompt"] = None
            items.append(current)
    return items


def judge_view(item: dict) -> str:
    """The compact text a judge reads for one item."""
    o = item["outcome"]
    tools = ", ".join(f"{k}×{v}" for k, v in list(o["tools"].items())[:6]) or "none"
    return (f"### {item['id']}\nCONTEXT: {clip(item['context'][-900:], 900) or '(start of session)'}\nPROMPT: {clip(item['prompt'], 700)}\n"
            f"OUTCOME: {o['toolCalls']} tool calls ({tools}); {o['filesEdited']} files edited; {o['outputTokens']} output tokens ({o['reasoningTokens']} reasoning); "
            f"{o['seconds']} s; effort {o['effort']}; {'INTERRUPTED; ' if o['interrupted'] else ''}final: {clip(o['final'], 200)!r}; next prompt: {clip(o['nextPrompt'] or '-', 100)!r}\n")


def main() -> None:
    state = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state")
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=state / "system1router/dataset")
    parser.add_argument("--shard-size", type=int, default=100)
    args = parser.parse_args()
    items = claude_items(Path.home() / ".claude/projects") + opencode_items(Path.home() / ".local/share/opencode/opencode.db")
    for item in items:
        item.pop("_start", None)
    args.out.mkdir(parents=True, exist_ok=True)
    os.chmod(args.out, 0o700)
    target = args.out / "items.jsonl"
    target.write_text("".join(json.dumps(item, ensure_ascii=False) + "\n" for item in items))
    os.chmod(target, 0o600)
    shards = args.out / "shards"
    shards.mkdir(exist_ok=True)
    for old in shards.glob("shard-*.md"):
        old.unlink()
    for index in range(0, len(items), args.shard_size):
        shard = shards / f"shard-{index // args.shard_size:02}.md"
        shard.write_text("".join(judge_view(item) + "\n" for item in items[index:index + args.shard_size]))
        os.chmod(shard, 0o600)
    by_source = collections.Counter(item["source"] for item in items)
    print(f"{len(items)} items {dict(by_source)} in {len(list(shards.glob('shard-*.md')))} shards -> {args.out}")
    print("projects:", dict(collections.Counter(item["project"] for item in items).most_common()))
    print("with context:", sum(bool(item["context"]) for item in items), "| interrupted:", sum(item["outcome"]["interrupted"] for item in items),
          "| efforts used:", dict(collections.Counter(item["outcome"]["effort"] for item in items).most_common()))
    print("judge chars total:", sum(len(judge_view(item)) for item in items))


if __name__ == "__main__":
    main()
