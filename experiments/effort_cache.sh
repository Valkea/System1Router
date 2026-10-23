#!/usr/bin/env bash
# What does an effort change cost Claude Code's prompt cache?
#
# Runs one short conversation with `claude -p --resume`, changing the effort
# between turns, and records each turn's usage. Two ways to change effort:
#   native: the session's own effort (`--effort`, what /effort sets)
#   mod:    the System1Router mod rewriting `turn.step`'s effort, the
#           session's own effort staying `medium`; the class is forced with
#           SYSTEM1ROUTER_JEV_RESPONSE, so no prompt leaves the machine
#           except to Anthropic.
#
# The first turn carries the project docs (~15k tokens) so the conversation,
# not just the system prompt and tools, is large enough for a lost messages
# cache to show.
#
# Usage: experiments/effort_cache.sh <out-dir> [model] [native|mod|both]
# Then:  python3 experiments/effort_cache_report.py <out-dir>
set -euo pipefail
out=$(mkdir -p "$1" && cd "$1" && pwd)
model=${2:-sonnet}
which=${3:-both}
repo=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$out/work" "$out/state"
cd "$out/work"

prompt='Reply with exactly one word: ok'
first="Here are a project's docs, for later questions.

$(cat "$repo/README.md" "$repo"/docs/*.md)

$prompt"

# turn <run> <label> <model> <effort> [extra claude args...]
turn() {
  local run=$1 label=$2 turn_model=$3 effort=$4; shift 4
  local resume=() text=$first
  [[ -f "$out/$run.sid" ]] && resume=(--resume "$(cat "$out/$run.sid")") text=$prompt
  claude -p --output-format json --model "$turn_model" --effort "$effort" --debug-file "$out/$run-$label.debug" "${resume[@]}" "$@" "$text" > "$out/$run-$label.json"
  [[ -f "$out/$run.sid" ]] || jq -r .session_id "$out/$run-$label.json" > "$out/$run.sid"
}

jev() {
  printf '{"answers":{"routing_class":{"choice":"%s","confidence":0.9},"task_type":{"choice":"debugging","confidence":0.9},"context":{"score":1,"confidence":0.9}}}' "$1"
}

if [[ $which == native || $which == both ]]; then
  rm -f "$out/native.sid"
  turn native 1-medium "$model" medium
  turn native 2-medium "$model" medium
  turn native 3-high "$model" high
  turn native 4-high "$model" high
  turn native 5-low "$model" low
  turn native 6-low "$model" low
  turn native 7-haiku-low haiku low
fi

if [[ $which == mod || $which == both ]]; then
  rm -f "$out/mod.sid"
  cat > "$out/config.json" <<'EOF'
{"routing":{"enabled":true,"swap":"effort"},"consent":{"mode":"always-accept"},
 "effort":{"cacheGuardTokens":0},
 "providers":{"selected":"jev","jev":{"baseUrl":"http://127.0.0.1:9","credentialEnv":null}}}
EOF
  export PATH="$repo/target/debug:$PATH" SYSTEM1ROUTER_CONFIG="$out/config.json" XDG_STATE_HOME="$out/state"
  # code_moderate -> high, question_simple -> low (default effort.levels)
  SYSTEM1ROUTER_JEV_RESPONSE=$(jev code_moderate) turn mod 1-high "$model" medium --plugin-dir "$repo/claude-code/system1router"
  SYSTEM1ROUTER_JEV_RESPONSE=$(jev code_moderate) turn mod 2-high "$model" medium --plugin-dir "$repo/claude-code/system1router"
  SYSTEM1ROUTER_JEV_RESPONSE=$(jev question_simple) turn mod 3-low "$model" medium --plugin-dir "$repo/claude-code/system1router"
  SYSTEM1ROUTER_JEV_RESPONSE=$(jev question_simple) turn mod 4-low "$model" medium --plugin-dir "$repo/claude-code/system1router"
  SYSTEM1ROUTER_JEV_RESPONSE=$(jev code_hard) turn mod 5-xhigh "$model" medium --plugin-dir "$repo/claude-code/system1router"
  SYSTEM1ROUTER_JEV_RESPONSE=$(jev code_hard) turn mod 6-xhigh "$model" medium --plugin-dir "$repo/claude-code/system1router"
fi
