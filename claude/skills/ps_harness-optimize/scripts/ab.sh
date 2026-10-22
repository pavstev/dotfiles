#!/usr/bin/env bash
# ab.sh — run one read-only prompt in two arms with `claude -p` and print turns, cost and time per run.
#   ab.sh --prompt TEXT --safe            [--runs N] [--model M] [--expect REGEX] [--dir DIR]
#   ab.sh --prompt TEXT --dirs DIR_A DIR_B [--runs N] [--model M] [--expect REGEX]
# --safe: arm A is DIR with the harness (CLAUDE.md, skills, hooks), arm B is the same DIR with `--safe-mode`.
# --dirs: both arms run normally, in two folders (for example a repo with and without a map file).
# --expect: a regex the answer must match; the table then shows hit or miss per run.
set -euo pipefail

prompt="" runs=3 model="sonnet" expect="" mode="" dir_a="$PWD" dir_b="" tools="Read Grep Glob"
while [ $# -gt 0 ]; do
  case "$1" in
    --prompt) prompt="$2"; shift 2 ;;
    --runs) runs="$2"; shift 2 ;;
    --model) model="$2"; shift 2 ;;
    --expect) expect="$2"; shift 2 ;;
    --tools) tools="$2"; shift 2 ;;
    --dir) dir_a="$2"; shift 2 ;;
    --safe) mode="safe"; shift ;;
    --dirs) mode="dirs"; dir_a="$2"; dir_b="$3"; shift 3 ;;
    *) echo "ab.sh: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$prompt" ] && [ -n "$mode" ] || { echo "usage: ab.sh --prompt TEXT (--safe | --dirs A B) [--runs N] [--model M] [--expect REGEX]" >&2; exit 2; }
command -v claude >/dev/null || { echo "ab.sh: claude is not on PATH" >&2; exit 127; }
if [ "$mode" = "safe" ] && ! claude --help 2>&1 | grep -q -- '--safe-mode'; then
  echo "ab.sh: this claude has no --safe-mode (added in 2.1.169). Update claude." >&2; exit 1
fi

field() { printf '%s' "$1" | grep -o "\"$2\":[^,}]*" | head -1 | sed -E 's/^[^:]*:"?//; s/"$//'; }

run_arm() { # arm dir extra-flag
  local arm="$1" dir="$2" flag="$3" i out turns cost ms hit
  for i in $(seq 1 "$runs"); do
    # shellcheck disable=SC2086
    out="$(cd "$dir" && claude -p "$prompt" --model "$model" --output-format json --allowedTools $tools $flag </dev/null 2>/dev/null || true)"
    turns="$(field "$out" num_turns)"; cost="$(field "$out" total_cost_usd)"; ms="$(field "$out" duration_ms)"
    hit="-"
    if [ -n "$expect" ]; then
      if printf '%s' "$out" | grep -Eq "$expect"; then hit="hit"; else hit="miss"; fi
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$arm" "$i" "${turns:-?}" "${cost:-?}" "$(( ${ms:-0} / 1000 ))" "$hit"
  done
}

table="$(mktemp)"
trap 'rm -f "$table"' EXIT
if [ "$mode" = "safe" ]; then
  { run_arm "A harness" "$dir_a" ""; run_arm "B safe-mode" "$dir_a" "--safe-mode"; } > "$table"
else
  { run_arm "A $(basename "$dir_a")" "$dir_a" ""; run_arm "B $(basename "$dir_b")" "$dir_b" ""; } > "$table"
fi
printf 'arm\trun\tturns\tcost_usd\tsecs\tanswer\n'
cat "$table"
echo "--- mean per arm (turns, cost_usd, secs)"
awk -F'\t' '$3 ~ /^[0-9.]+$/ && $4 ~ /^[0-9.]+$/ {t[$1]+=$3; c[$1]+=$4; s[$1]+=$5; n[$1]++}
  END {for (a in n) printf "%s\t%.1f\t%.4f\t%.0f\t(%d runs)\n", a, t[a]/n[a], c[a]/n[a], s[a]/n[a], n[a]}' "$table" | sort
