#!/usr/bin/env bash
# Claude Code status line — copper-themed. Receives session JSON on stdin.
# Shows:   <dir>   <git branch>   <model>   ctx <K> <%>   5h <%> · 7d <%>
# Context turns copper at 300 K: time to /ps:handoff. Rate limits are absent
# until the first API reply; then nothing is printed for them.
set -euo pipefail

input="$(cat)"
# Without jq nothing below works: print the folder name and stop.
if ! command -v jq >/dev/null 2>&1; then
  printf ' %s' "$(basename "$PWD")"
  exit 0
fi
get() { printf '%s' "$input" | jq -r "$1 // empty" 2>/dev/null; }

# Pace cache for claude/hooks/pace.py: written atomically when the weekly limit is present.
pace_file="${XDG_CACHE_HOME:-$HOME/.cache}/claude-pace.json"
if [ -n "$(get '.rate_limits.seven_day.used_percentage')" ] && mkdir -p "$(dirname "$pace_file")" 2>/dev/null; then
  pace_tmp="$pace_file.$$"
  if printf '%s' "$input" | jq -c --argjson now "$(date +%s)" '{updated: $now, five_hour: (.rate_limits.five_hour | if . then {used_percentage, resets_at} else null end), seven_day: (.rate_limits.seven_day | {used_percentage, resets_at})}' > "$pace_tmp" 2>/dev/null; then
    mv "$pace_tmp" "$pace_file" 2>/dev/null || rm -f "$pace_tmp"
  else
    rm -f "$pace_tmp"
  fi
fi

cwd="$(get '.workspace.current_dir')"; [ -n "$cwd" ] || cwd="$(get '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
model="$(get '.model.display_name')"
dir="$(basename "$cwd")"
branch="$(git -C "$cwd" branch --show-current 2>/dev/null || true)"
# Current context = the last call's input + cache tokens (total_input_tokens is a session running total).
ctx_tok="$(get '.context_window.current_usage | (.input_tokens + .cache_creation_input_tokens + .cache_read_input_tokens)')"
[ -n "$ctx_tok" ] && [ "$ctx_tok" != "0" ] || ctx_tok="$(get '.context_window.total_input_tokens')"
ctx_pct="$(get '.context_window.used_percentage')"
five="$(get '.rate_limits.five_hour.used_percentage')"
week="$(get '.rate_limits.seven_day.used_percentage')"

# Refined Claude palette (truecolor).
copper=$'\033[38;2;224;130;95m'   # #E0825F
sage=$'\033[38;2;135;176;131m'    # #87B083
stone=$'\033[38;2;168;162;158m'   # #A8A29E
rst=$'\033[0m'

int() { printf '%.0f' "$1" 2>/dev/null || true; }

line="${copper} ${dir}${rst}"
[ -n "$branch" ] && line+="  ${sage} ${branch}${rst}"
[ -n "$model" ]  && line+="  ${stone}${model}${rst}"

if [ -n "$ctx_tok" ]; then
  k=$(( $(int "$ctx_tok") / 1000 ))
  col="$stone"; [ "$k" -ge 300 ] && col="$copper"
  ctx="ctx ${k}K"; [ -n "$ctx_pct" ] && ctx+=" $(int "$ctx_pct")%"
  line+="  ${col}${ctx}${rst}"
fi

caps=""
[ -n "$five" ] && caps="5h $(int "$five")%"
[ -n "$week" ] && caps+="${caps:+ · }7d $(int "$week")%"
[ -n "$caps" ] && line+="  ${stone}${caps}${rst}"
printf '%s' "$line"
