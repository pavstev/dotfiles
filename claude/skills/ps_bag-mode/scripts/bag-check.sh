#!/usr/bin/env bash
# bag-check: is this Mac safe to put in a bag with Claude still running?
#   bag-check.sh [--eta 45m]    one compact card (≈3s). ETA = how long the session still needs.
#   bag-check.sh --throttle     also drop dev servers / indexers to background priority (reversible)
#   bag-check.sh --restore      undo --throttle
#   bag-check.sh --watch [N]    repeat the card every N seconds (default 60); the ETA counts down
# ETA formats: 45  45m  2h  1h30m. Always ends with one "VERDICT:" line.
set -uo pipefail
if [ "$(uname -s)" != Darwin ]; then
  echo "bag-mode is for a Mac laptop (it reads pmset, sysctl and Amphetamine). Nothing to check on $(uname -s)."
  echo "VERDICT: NOT APPLICABLE (not a Mac)"
  exit 0
fi
PS=/bin/ps
NCPU=$(sysctl -n hw.ncpu)
HEAVY=${BAG_HEAVY_CPU:-15}   # % of ONE core that counts as "heavy"
MAXROWS=5                    # heavy rows shown before "+N more"
BUFFER=25                    # % safety margin added to the ETA
START=$(date +%s)

parse_min() { # "45" | "45m" | "2h" | "1h30m" -> minutes
  echo "$1" | tr 'A-Z' 'a-z' | awk '{
    h=0; m=0
    if (match($0,/[0-9]+h/)) h=substr($0,RSTART,RLENGTH-1)
    s=$0; sub(/[0-9]+h/,"",s)
    if (match(s,/[0-9]+/)) m=substr(s,RSTART,RLENGTH)
    print h*60+m }'
}
fmt_min() { awk -v m="$1" 'BEGIN{ if (m>=60) printf "%dh%02dm", m/60, m%60; else printf "%dm", m }'; }

classify() { # <command> -> "emoji kind|when it ends"
  case "$1" in
    *WindowServer*)                         echo "🖥  screen drawing|when the lid closes";;
    *"Claude Helper (Renderer)"*|*Claude.app/Contents/MacOS/Claude) echo "🪟 Claude window|when it is hidden";;
    *claude-code*|*/claude|*claude.app/*)   echo "🤖 Claude agent|when its task ends";;
    *next-server*|*vite*|*webpack*|*workerd*|*wrangler*|*astro*) echo "🛠  dev server|never - stop it";;
    *rustc*|*cargo*|*tsc*|*esbuild*|*swc*|*biome*|*turbo*|*pnpm*) echo "🔨 build/lint|when the build ends";;
    *dockerd*|*com.docker*|*colima*|*qemu*|*vz*) echo "🐳 container VM|never - stop if idle";;
    *mds*|*mdworker*|*mds_stores*)          echo "🔍 Spotlight|in a few minutes";;
    *brew*|*softwareupdate*|*backupd*)      echo "🧰 system job|ends alone";;
    *node*|*bun*|*deno*|*python*)           echo "📜 script ($(basename "$1"))|unknown";;
    *)                                      echo "❔ $(basename "$1")|unknown";;
  esac
}

# Usage: pids of things safe to throttle (dev servers + indexers).
throttle_pids() { $PS -Ao pid,comm | awk '/next-server|workerd|vite|webpack|wrangler|mdworker|mds_stores/ {print $1}'; }

report() {
  local lvl=0 why="" bump
  bump() { [ "$1" -gt "$lvl" ] && { lvl=$1; why=$2; }; }  # keep the worst level and its reason

  # --- power
  local b src pct rem_min amph
  b=$(pmset -g batt)
  src=$(sed -n 1p <<<"$b" | grep -o "'[^']*'" | tr -d "'" | sed 's/ Power//')
  pct=$(grep -o '[0-9]*%' <<<"$b" | head -1)
  rem_min=$(grep -o '[0-9]*:[0-9][0-9] remaining' <<<"$b" | awk '{split($1,a,":"); print a[1]*60+a[2]}')
  [ "${rem_min:-0}" -eq 0 ] 2>/dev/null && rem_min=""   # "0:00 remaining" = no estimate yet, not zero minutes
  if pmset -g assertions | grep -q 'Amphetamine'; then amph="✅ ON"
  else amph="❌ OFF - the Mac will sleep"; bump 2 "Amphetamine is OFF - the Mac will sleep"; fi
  local pw="${pct:-?} on ${src:-?}"
  [ "$src" = "Battery" ] && [ -n "$rem_min" ] && pw="$pw · $(fmt_min "$rem_min") left"

  # --- load
  local l1 memfree
  l1=$(sysctl -n vm.loadavg | awk '{print $2}')
  memfree=$(memory_pressure 2>/dev/null | awk '/free percentage/ {print $NF}')
  if   awk -v l="$l1" -v n="$NCPU" 'BEGIN{exit !(l>n)}';   then bump 2 "load above core count - runs hot in a bag"
  elif awk -v l="$l1" -v n="$NCPU" 'BEGIN{exit !(l>n/2)}'; then bump 1 "busy - use --throttle"; fi

  # --- ETA
  local eta_line="unknown - pass --eta 45m"
  if [ -n "${ETA_MIN:-0}" ] && [ "${ETA_MIN:-0}" -gt 0 ]; then
    local el eff need
    el=$(( ($(date +%s) - START) / 60 ))
    eff=$(( ETA_MIN - el )); [ "$eff" -lt 0 ] && eff=0
    need=$(( eff + eff * BUFFER / 100 ))
    eta_line="~$(fmt_min "$eff") (+${BUFFER}% buffer = $(fmt_min "$need"))"
    if [ "$src" = "Battery" ] && [ -n "$rem_min" ]; then
      if [ "$rem_min" -lt "$need" ]; then eta_line="$eta_line · ⚠️ battery short"; bump 2 "battery ($(fmt_min "$rem_min")) is less than ETA - plug in"
      else eta_line="$eta_line · ✅ battery ok"; fi
    else eta_line="$eta_line · ✅ power ok"; fi
  elif [ "$src" = "Battery" ] && [ "${pct%\%}" -lt 20 ] 2>/dev/null; then
    bump 1 "battery under 20% - plug in"
  fi

  # --- heavy
  local rows n
  rows=$(top -l 2 -n 25 -o cpu -stats pid,cpu 2>/dev/null | awk '/^PID/ {n++} n==2 && $1 ~ /^[0-9]+$/ {print $1, $2}' |
  while read -r pid cpu; do
    awk -v c="$cpu" -v h="$HEAVY" 'BEGIN{exit !(c>=h)}' || continue
    cmd=$($PS -p "$pid" -o comm= 2>/dev/null) || continue
    IFS='|' read -r kind dur <<<"$(classify "$cmd")"
    printf '| %s | %.0f%% | %s |\n' "$kind" "$cpu" "$dur"
  done)
  n=$(grep -c . <<<"$rows")

  # --- counts
  local ag ds nd
  ag=$($PS -Ao comm | grep -c 'claude.app/Contents/MacOS/claude')
  ds=$($PS -Ao comm | grep -cE 'next-server|workerd|vite|webpack')
  nd=$($PS -Ao comm | grep -cE '(^|/)node$')

  # --- verdict text
  local lname lemoji msg
  case $lvl in
    0) lname=LOW;    lemoji=🟢; msg="safe to close the lid.";;
    1) lname=MEDIUM; lemoji=🟡; msg="$why.";;
    *) lname=HIGH;   lemoji=🔴; msg="$why.";;
  esac

  # --- card (markdown: renders as tables in Claude)
  printf '## 🎒 Bag Mode — %s %s\n\n' "$lemoji" "$lname"
  printf '| Check | Status |\n|---|---|\n'
  printf '| 🔋 Power | %s |\n' "$pw"
  printf '| ☕ Amphetamine | %s |\n' "$amph"
  printf '| 🔥 Load | %s / %s cores · %s memory free |\n' "$l1" "$NCPU" "${memfree:-?}"
  printf '| ⏱ ETA | %s |\n' "$eta_line"
  printf '| 🤖 Running | %s agents · %s dev servers · %s node |\n' "$ag" "$ds" "$nd"
  printf '\n### 🏋 Heavy tasks (%s)\n\n' "${n:-0}"
  if [ "${n:-0}" -gt 0 ]; then
    printf '| Task | CPU | Ends |\n|---|---|---|\n'
    head -n "$MAXROWS" <<<"$rows"
    [ "$n" -gt "$MAXROWS" ] && printf '| … +%s more | | |\n' $((n - MAXROWS))
  else
    printf 'None. Nothing is above %s%% of a core. ✨\n' "$HEAVY"
  fi
  printf '\nVERDICT: %s %s - %s\n' "$lemoji" "$lname" "$msg"
}

MODE=report; INTERVAL=60; ETA_MIN=""
while [ $# -gt 0 ]; do
  case "$1" in
    --eta)      ETA_MIN=$(parse_min "${2:-}"); shift;;
    --throttle) MODE=throttle;;
    --restore)  MODE=restore;;
    --watch)    MODE=watch; case "${2:-}" in ''|--*) ;; *) INTERVAL=$2; shift;; esac;;
    *)          echo "unknown option: $1" >&2; exit 2;;
  esac
  shift
done

case "$MODE" in
  throttle) for p in $(throttle_pids); do taskpolicy -b -p "$p" 2>/dev/null && echo "throttled $p"; done; report;;
  restore)  for p in $(throttle_pids); do taskpolicy -B -p "$p" 2>/dev/null && echo "restored $p"; done;;
  watch)    while :; do clear; date; report; sleep "$INTERVAL"; done;;
  *)        report;;
esac
