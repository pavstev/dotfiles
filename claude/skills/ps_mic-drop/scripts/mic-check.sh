#!/usr/bin/env bash
# mic-check: is this repo really done? Read-only (except `git fetch`).
#   mic-check.sh [repo...]         report for each repo (default: current repo)
#   mic-check.sh --no-fetch        skip `git fetch` (offline / fast)
#   mic-check.sh --note "text" [repo]  append an optional leftover (tagged with that repo, default: current)
#   mic-check.sh --leftovers       print the leftovers file
#   mic-check.sh --since <sha|time> [repo]  also list this session's commits since that sha or time (for squash)
# Every line is tagged:
#   OK    nothing to do
#   FIX   mandatory — the session is NOT done until this is gone
#   WARN  look at it; fix or note it as a leftover
# Last line: VERDICT: CLEAN (exit 0) | NOT CLEAN (n mandatory) (exit 1)
set -uo pipefail
LEFTOVERS=${MIC_LEFTOVERS:-$HOME/.claude/leftovers.md}
FETCH=1
SINCE=
REPOS=()
fixes=0

ROOT=.
g()    { git -C "$ROOT" "$@"; }
ok()   { printf '  OK    %s\n' "$*"; }
fix()  { printf '  FIX   %s\n' "$*"; fixes=$((fixes + 1)); }
warn() { printf '  WARN  %s\n' "$*"; }

note() { # <text> [dir] -> append under today's date + the repo name of dir (default: current dir)
  local dir=${2:-$PWD} root name
  root=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) && name=$(basename "$root") || name=$(basename "$dir")
  [ -f "$LEFTOVERS" ] || printf '# Leftovers\n\nOptional fixes saved by ps:mic-drop. Delete a line when done.\n' >"$LEFTOVERS"
  printf -- '- [ ] %s · **%s** · %s\n' "$(date +%F)" "$name" "$1" >>"$LEFTOVERS"
  echo "saved to $LEFTOVERS"
}

checks_for() { # print the verification commands this repo offers
  local r=$1 t
  if [ -f "$r/package.json" ]; then
    local pm=npm
    [ -f "$r/pnpm-lock.yaml" ] && pm=pnpm
    [ -f "$r/bun.lockb" ] || [ -f "$r/bun.lock" ] && pm=bun
    [ -f "$r/yarn.lock" ] && pm=yarn
    for t in typecheck lint test check build; do
      grep -q "\"$t\"[[:space:]]*:" "$r/package.json" && echo "$pm run $t"
    done
  fi
  if [ -f "$r/Makefile" ]; then
    for t in check lint test verify; do grep -qE "^$t:" "$r/Makefile" && echo "make $t"; done
  fi
  if [ -f "$r/justfile" ] || [ -f "$r/Justfile" ]; then
    for t in check lint test verify; do just --justfile "$r/"[jJ]ustfile --summary 2>/dev/null | tr ' ' '\n' | grep -qx "$t" && echo "just $t"; done
  fi
  [ -f "$r/mise.toml" ] && grep -qE '^\[tasks\.(check|lint|test)\]' "$r/mise.toml" && echo "mise tasks ls  # then run check/lint/test"
  [ -f "$r/.pre-commit-config.yaml" ] && echo "pre-commit run --all-files"
  [ -f "$r/Cargo.toml" ] && echo "cargo clippy --all-targets && cargo nextest run"
  [ -f "$r/go.mod" ] && echo "golangci-lint run && go test ./..."
  [ -f "$r/pyproject.toml" ] && echo "uv run ruff check && uv run ruff format --check && uv run pytest"
  [ -f "$r/biome.json" ] || [ -f "$r/biome.jsonc" ] && echo "biome check ."
  local hp; hp=$(git -C "$r" config core.hooksPath)
  { [ -n "$hp" ] && [ -x "$r/$hp/pre-commit" ]; } || [ -x "$(git -C "$r" rev-parse --git-path hooks/pre-commit)" ] &&
    echo "git hook run pre-commit  # this repo's own pre-commit hook"
  ls "$r"/.github/workflows/*.y*ml >/dev/null 2>&1 && echo "(CI) gh run list --limit 3"
  return 0
}

# A command line can hold keys: `--api-key X`, KEY=VALUE env dumps in a process
# title, a password in a URL. safe_args masks them (over-masking is fine).
safe_args() {
  tr '\n\t' '  ' | sed -E \
    -e 's#(://)[^/@[:space:]]+@#\1***@#g' \
    -e 's/(--?[A-Za-z0-9_.-]*(key|token|secret|passw|pwd|auth|credential|cookie|session)[A-Za-z0-9_.-]*)(=|[[:space:]]+)[^[:space:]]+/\1\3***/gI' \
    -e 's/([A-Za-z0-9_.-]*(key|token|secret|passw|pwd|auth|credential|cookie|session)[A-Za-z0-9_.-]*)=[^[:space:]]+/\1=***/gI' \
    -e 's/(bearer|basic)[[:space:]]+[^[:space:]]+/\1 ***/gI' \
    -e 's/(crsr_|sk-|sk_|ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|xox[abprs]-|AKIA|AIza|ya29\.)[A-Za-z0-9_.-]+/\1***/g' \
    -e 's/[A-Za-z0-9_+]{32,}/***/g' \
    -e 's/[[:space:]]+/ /g' -e 's/^ //' -e 's/ $//'
}

proc_line() { # <pid> -> "pid name: args", args masked and cut to 100 chars; nothing for mic-check itself
  local args name
  args=$(ps -o args= -p "$1" 2>/dev/null) || return 0
  case "$args" in ''|*mic-check*) return 0;; esac
  name=$(ps -o ucomm= -p "$1" 2>/dev/null | sed 's/[[:space:]]*$//')
  args=$(safe_args <<<"$args")
  [ ${#args} -gt 100 ] && args="${args:0:100}…"
  printf '%s %s: %s\n' "$1" "${name:-?}" "$args"
}

report() { # <dir>
  local root
  root=$(git -C "$1" rev-parse --show-toplevel 2>/dev/null) || { echo "== $1 =="; warn "not a git repo — skipped"; return; }
  ROOT=$root; local gd branch main up
  gd=$(g rev-parse --absolute-git-dir)
  echo "== $(basename "$root")  ($root) =="

  [ "$FETCH" = 1 ] && { g fetch --quiet --prune 2>/dev/null || warn "git fetch failed — remote state may be old"; }

  branch=$(g symbolic-ref --quiet --short HEAD) || branch=""
  main=$(g symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null | sed 's#^origin/##')
  [ -z "$main" ] && { g show-ref --verify --quiet refs/remotes/origin/main && main=main || main=master; }

  # 1. Merge / rebase in progress, conflicts
  local state=""
  [ -f "$gd/MERGE_HEAD" ] && state="merge"
  [ -d "$gd/rebase-merge" ] || [ -d "$gd/rebase-apply" ] && state="rebase"
  [ -f "$gd/CHERRY_PICK_HEAD" ] && state="cherry-pick"
  [ -f "$gd/REVERT_HEAD" ] && state="revert"
  [ -n "$state" ] && fix "a $state is half done — finish it or abort it"
  local conflicts
  conflicts=$(g diff --name-only --diff-filter=U | wc -l | tr -d ' ')
  [ "$conflicts" -gt 0 ] && fix "$conflicts file(s) with merge conflicts: $(g diff --name-only --diff-filter=U | head -5 | tr '\n' ' ')"
  local markers
  markers=$(g grep -l -E '^(<<<<<<<|>>>>>>>) ' 2>/dev/null | head -3 | tr '\n' ' ')
  [ -n "$markers" ] && fix "conflict markers in files: $markers"
  [ -z "$state" ] && [ "$conflicts" -eq 0 ] && ok "no conflicts, no half-done merge/rebase"

  # 2. Branch
  if [ -z "$branch" ]; then fix "detached HEAD — not on a branch"
  elif [ "$branch" = "$main" ]; then ok "on $main"
  else warn "on branch '$branch', not '$main' — merge it, or open a PR (ask the user)"; fi

  # 3. Uncommitted work
  local staged unstaged untracked
  staged=$(g diff --cached --name-only | wc -l | tr -d ' ')
  unstaged=$(g diff --name-only | wc -l | tr -d ' ')
  untracked=$(g ls-files --others --exclude-standard | wc -l | tr -d ' ')
  if [ $((staged + unstaged + untracked)) -eq 0 ]; then ok "working tree clean"
  else fix "not committed: $staged staged, $unstaged changed, $untracked new file(s)"
       g status --short | head -8 | sed 's/^/          /'; fi

  # 4. Pushed?
  if [ -n "$branch" ]; then
    up=$(g rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null) || up=""
    if [ -z "$up" ]; then fix "branch '$branch' has no upstream — never pushed"
    else
      local ahead behind
      read -r behind ahead < <(g rev-list --left-right --count "$up...HEAD")
      [ "$ahead" -gt 0 ] && fix "$ahead commit(s) not pushed to $up"
      [ "$behind" -gt 0 ] && fix "$behind commit(s) on $up not pulled (pull --rebase, then re-check)"
      [ "$ahead" -eq 0 ] && [ "$behind" -eq 0 ] && ok "in sync with $up ($(g rev-parse --short HEAD))"
    fi
  fi
  if [ -n "$branch" ] && [ "$branch" != "$main" ] && g show-ref --verify --quiet "refs/remotes/origin/$main"; then
    local notin
    notin=$(g rev-list --count "origin/$main..HEAD")
    [ "$notin" -gt 0 ] && warn "$notin commit(s) on '$branch' are not in origin/$main yet"
  fi

  # 4b. This session's commits (squash mode): only with --since <sha|time>
  if [ -n "$SINCE" ] && [ -n "$branch" ]; then
    local rng first base n me foreign
    if [[ "$SINCE" =~ ^[0-9a-fA-F]{7,40}$ ]]; then  # a sha: must be in this branch, never read as a date
      if g merge-base --is-ancestor "$SINCE" HEAD 2>/dev/null; then base=$(g rev-parse --verify --quiet "$SINCE^{commit}^" || true); rng=${base:+$base..HEAD}  # inclusive: the base is the parent
      else warn "--since $SINCE is not an ancestor of HEAD here: no squash range for this repo"; base=""; rng=""; fi
    else first=$(g rev-list --since="$SINCE" HEAD | tail -1)
         base=$([ -n "$first" ] && g rev-parse --verify --quiet "$first^" || true); rng=${base:+$base..HEAD}; fi
    n=$([ -n "$rng" ] && g rev-list --count "$rng" || echo 0)
    me=$(g config user.email)
    foreign=$([ -n "$rng" ] && g log --format='%ae' "$rng" | grep -vcxF "$me" || true)
    if [ "$n" -eq 0 ]; then ok "no session commits since $SINCE (or the base is not found)"
    else ok "$n session commit(s) on top of base $(g rev-parse --short "$base") (squash: git reset --soft $(g rev-parse --short "$base"))"
         g log --format='          %h %an  %s' "$rng" | head -8
         [ "${foreign:-0}" -gt 0 ] && warn "$foreign of those commit(s) have another author — do not squash them"
         [ -n "$(g rev-list --merges "$rng")" ] && warn "merge commit(s) in that range — do not squash"; fi
  fi

  # 5. Other local branches with unpushed work
  local b
  g for-each-ref --format='%(refname:short)|%(upstream:short)|%(upstream:track)' refs/heads |
  while IFS='|' read -r b u t; do
    [ "$b" = "$branch" ] && continue
    if [ -z "$u" ]; then
      g merge-base --is-ancestor "$b" "origin/$main" 2>/dev/null || warn "branch '$b' is local only and not merged"
    elif [[ "$t" == *ahead* ]]; then warn "branch '$b' $t vs $u"; fi
  done

  # 6. Stashes and extra worktrees
  local s w
  s=$(g stash list | wc -l | tr -d ' ')
  [ "$s" -gt 0 ] && warn "$s stash(es) — forgotten work? ($(g stash list | head -1))"
  w=$(g worktree list | wc -l | tr -d ' ')
  [ "$w" -gt 1 ] && warn "$((w - 1)) extra worktree(s): $(g worktree list | tail -n +2 | awk '{print $1}' | tr '\n' ' ')"

  # 7. CI for the latest pushed commit
  if command -v gh >/dev/null && ls "$root"/.github/workflows/*.y*ml >/dev/null 2>&1 && [ -n "$branch" ]; then
    local head runs
    head=$(g rev-parse HEAD)
    runs=$(cd "$root" && gh run list --commit "$head" --json name,status,conclusion \
           -q '.[] | "\(.status)|\(.conclusion)|\(.name)"' 2>/dev/null) || runs="ERR"
    if [ "$runs" = "ERR" ]; then warn "could not read CI (gh auth?)"
    elif [ -z "$runs" ]; then warn "no CI run found for $(g rev-parse --short HEAD) (not pushed yet, or CI not triggered)"
    else
      local bad pending
      bad=$(grep -E '^completed\|(failure|cancelled|timed_out|startup_failure)' <<<"$runs" | cut -d'|' -f3 | sort -u | tr '\n' ' ')
      pending=$(grep -vc '^completed' <<<"$runs")
      [ -n "$bad" ] && fix "CI failed on HEAD: $bad"
      [ "$pending" -gt 0 ] && warn "$pending CI run(s) still running on HEAD — wait, then re-check"
      [ -z "$bad" ] && [ "$pending" -eq 0 ] && ok "CI green on HEAD"
    fi
  fi

  # 8. Verification checks this repo offers (the agent runs them)
  local c
  c=$(checks_for "$root")
  if [ -n "$c" ]; then echo "  RUN   verification checks for this repo:"; sed 's/^/          /' <<<"$c"
  else warn "no known check command found — ask how this repo is verified, or check README/AGENTS.md"; fi

  # 9. Stray processes started from this repo (dev servers, watchers): one short,
  # masked line each (proc_line), never the raw command line.
  local p pid
  p=$(for pid in $(pgrep -f "$root" 2>/dev/null); do proc_line "$pid"; done | head -5)
  [ -n "$p" ] && { warn "processes still running from this repo:"; sed 's/^/          /' <<<"$p"; }
  echo
}

while [ $# -gt 0 ]; do
  case "$1" in
    --no-fetch)  FETCH=0;;
    --since)     shift; SINCE=${1:?--since needs a sha or time};;
    --note)      shift; NOTE=${1:?--note needs text};;
    --leftovers) cat "$LEFTOVERS" 2>/dev/null || echo "no leftovers file yet ($LEFTOVERS)"; exit 0;;
    -h|--help)   sed -n '2,10p' "$0"; exit 0;;
    *)           REPOS+=("$1");;
  esac
  shift
done
[ -n "${NOTE:-}" ] && { note "$NOTE" "${REPOS[0]:-$PWD}"; exit 0; }
[ ${#REPOS[@]} -eq 0 ] && REPOS=("$PWD")

for r in "${REPOS[@]}"; do report "$r"; done
if [ -f "$LEFTOVERS" ]; then
  echo "leftovers file: $LEFTOVERS ($(grep -c '^- \[ \]' "$LEFTOVERS") open)"
fi
if [ "$fixes" -eq 0 ]; then echo "VERDICT: CLEAN"; else echo "VERDICT: NOT CLEAN ($fixes mandatory)"; exit 1; fi
