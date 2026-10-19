#!/usr/bin/env bash
# ============================================================================
#  scripts/test-linux.sh <image> — run ./dotfiles end to end in a fresh Linux
#  container, twice, and check the result.
#
#    bash scripts/test-linux.sh debian:13
#    bash scripts/test-linux.sh ubuntu:24.04
#
#  Needs podman or docker on the host that runs it. No sudo on the host.
#  What it does:
#    1. builds a base image once (image + sudo + git + curl + a user "dev" with sudo),
#       like a fresh install that has nothing else
#    2. starts a container, copies this working tree in as a git repo with no remote
#       (git-ignored files stay out), pipes a git name and email to ./dotfiles
#    3. run 1: ./dotfiles. Run 2: ./dotfiles again, which must change nothing
#    4. checks: links, tools, `zsh -ic exit` prints nothing, bash -n, the unit tests
#    5. removes the container (the base image stays, for speed)
#  Exit 0 only when every check passes.
#
#  Options:  --keep   leave the container running (shell in: podman exec -it NAME bash)
#            --reuse  use the kept container: sync the tree, run both runs again
#            --clean  also remove the base image at the end
#  Env:      CONTAINER_RUNTIME=podman|docker (default: the first one found)
#            TEST_LOG_DIR=<dir> for the logs (default: $TMPDIR/dots-test-linux)
#  Debian and Ubuntu images only (apt).
# ============================================================================
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="" KEEP=0 REUSE=0 CLEAN=0
for a in "$@"; do
  case "$a" in
    --keep) KEEP=1 ;;
    --reuse) REUSE=1; KEEP=1 ;;
    --clean) CLEAN=1 ;;
    -h|--help) sed -n '2,28p' "${BASH_SOURCE[0]}"; exit 0 ;;
    -*) printf 'unknown option: %s\n' "$a" >&2; exit 2 ;;
    *) IMAGE="$a" ;;
  esac
done
[[ -n "$IMAGE" ]] || { printf 'usage: test-linux.sh <image> [--keep] [--reuse] [--clean]\n' >&2; exit 2; }

RT="${CONTAINER_RUNTIME:-}"
if [[ -z "$RT" ]]; then
  for c in podman docker; do command -v "$c" >/dev/null 2>&1 && { RT="$c"; break; }; done
fi
[[ -n "$RT" ]] || { printf 'no podman or docker found\n' >&2; exit 2; }

slug="$(printf '%s' "$IMAGE" | tr -c 'a-zA-Z0-9\n' '-' | sed 's/-*$//')"
BASE="dots-test-base-$slug"
NAME="dots-test-$slug"
LOGS="${TEST_LOG_DIR:-${TMPDIR:-/tmp}/dots-test-linux-$(id -u)}/$slug"
(umask 077; mkdir -p "$LOGS")
[[ -d "$LOGS" && ! -L "$LOGS" ]] || { printf 'bad log dir: %s\n' "$LOGS" >&2; exit 2; }
rm -f "$LOGS"/*.log

fails=0
pass() { printf 'PASS  %s\n' "$*"; }
bad()  { printf 'FAIL  %s\n' "$*"; fails=$((fails + 1)); }
note() { printf '»»    %s\n' "$*"; }
cx()   { "$RT" exec -i "$NAME" bash -lc "$1"; }   # a command as the user "dev"

cleanup() {
  if (( KEEP == 0 )); then "$RT" rm -f "$NAME" >/dev/null 2>&1; fi
  if (( CLEAN == 1 )); then "$RT" rmi -f "$BASE" >/dev/null 2>&1; fi
}
trap cleanup EXIT

# ---- 1. base image ----------------------------------------------------------
if ! "$RT" image inspect "$BASE" >/dev/null 2>&1; then
  note "building $BASE from $IMAGE"
  ctx="$(mktemp -d)"
  if ! "$RT" build -q -t "$BASE" -f - "$ctx" >"$LOGS/build.log" 2>&1 <<EOF
FROM $IMAGE
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends sudo ca-certificates curl git \
 && rm -rf /var/lib/apt/lists/*
RUN useradd -m -s /bin/bash dev \
 && echo 'dev ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/dev && chmod 440 /etc/sudoers.d/dev
EOF
  then
    rmdir "$ctx" 2>/dev/null
    bad "base image build failed (see $LOGS/build.log)"
    exit 1
  fi
  rmdir "$ctx" 2>/dev/null
fi

# ---- 2. container and working tree ------------------------------------------
if (( REUSE == 1 )) && "$RT" container inspect "$NAME" >/dev/null 2>&1; then
  note "reusing container $NAME"
else
  "$RT" rm -f "$NAME" >/dev/null 2>&1
  "$RT" run -d --name "$NAME" --user dev -w /home/dev "$BASE" sleep infinity >/dev/null \
    || { bad "couldn't start the container"; exit 1; }
fi

# The files git would commit or list as new: ignored files (secrets) stay out.
# Without git here (a copy of the tree), take everything except local state.
export COPYFILE_DISABLE=1
if git -C "$REPO" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  # (a tracked file deleted in the working tree is left out)
  pack() {
    git -C "$REPO" ls-files -z --cached --others --exclude-standard |
      while IFS= read -r -d '' f; do [[ -e "$REPO/$f" ]] && printf '%s\0' "$f"; done |
      tar -C "$REPO" --null -T - -cf -
  }
else
  pack() { tar -C "$REPO" --exclude=./.git --exclude=./.agents --exclude=./tmp \
             --exclude=__pycache__ --exclude=local.zsh -cf - .; }
fi
cx 'mkdir -p ~/dotfiles && find ~/dotfiles -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +'
pack | "$RT" exec -i "$NAME" bash -lc 'tar -xf - -C ~/dotfiles' || { bad "couldn't copy the tree"; exit 1; }
cx 'cd ~/dotfiles && git init -q -b main 2>/dev/null; git add -A &&
    git -c core.hooksPath=/dev/null -c user.name=test -c user.email=test@example.com commit -q --allow-empty -m snapshot' \
  || { bad "couldn't make the snapshot repo"; exit 1; }
# A local "remote", so the pull at the top of ./dotfiles works like it does on a real machine.
cx 'rm -rf ~/origin.git && git clone -q --bare ~/dotfiles ~/origin.git && cd ~/dotfiles &&
    git remote remove origin 2>/dev/null; git remote add origin ~/origin.git &&
    git fetch -q origin && git branch -q -u origin/main' \
  || { bad "couldn't set up the local remote"; exit 1; }

# A fingerprint of what ./dotfiles manages, to prove run 2 changes nothing.
STATE='cd ~; { find ~ -maxdepth 5 -type l -printf "%p -> %l\n" 2>/dev/null | sort
  ls ~/.oh-my-zsh/custom/plugins ~/.local/bin ~/.config/dotfiles 2>/dev/null
  cat ~/.gitconfig.local ~/.config/dotfiles/settings 2>/dev/null
  dpkg-query -W -f "\${Package}\n" 2>/dev/null | sort
  export PATH="/home/linuxbrew/.linuxbrew/bin:$HOME/.linuxbrew/bin:$HOME/.cargo/bin:$PATH"
  brew list --formula 2>/dev/null | sort
  rustup toolchain list 2>/dev/null; mise ls 2>/dev/null
  gh extension list 2>/dev/null; } | md5sum'

# ---- 3. run 1 and run 2 ------------------------------------------------------
note "run 1 (fresh): ./dotfiles on $IMAGE ($RT, logs in $LOGS)"
t0=$SECONDS
printf 'Test User\ntest@example.com\n' | "$RT" exec -i -e NO_COLOR=1 -e DOTFILES_PROFILE=shared "$NAME" \
  bash -lc 'cd ~/dotfiles && ./dotfiles' >"$LOGS/run1.log" 2>&1
rc1=$?
note "run 1 took $((SECONDS - t0))s, exit $rc1"
if (( rc1 == 0 )); then pass "run 1 exits 0"; else bad "run 1 exit $rc1"; fi

# Warnings go to stderr as "!  text". A clean run has none.
unexpected="$(grep -E '^!  ' "$LOGS/run1.log" || true)"
if [[ -z "$unexpected" ]]; then pass "run 1 prints no unexpected warnings"
else bad "run 1 warnings:"; printf '%s\n' "$unexpected" | sed 's/^/        /'; fi

before="$(cx "$STATE")"
note "run 2 (same container): ./dotfiles again"
t0=$SECONDS
"$RT" exec -i -e NO_COLOR=1 -e DOTFILES_PROFILE=shared "$NAME" bash -lc 'cd ~/dotfiles && ./dotfiles' \
  </dev/null >"$LOGS/run2.log" 2>&1
rc2=$?
note "run 2 took $((SECONDS - t0))s, exit $rc2"
if (( rc2 == 0 )); then pass "run 2 exits 0"; else bad "run 2 exit $rc2"; fi
after="$(cx "$STATE")"
changes="$(grep -E '^(✓ (linked|removed)|»» (Installing|installing|Initialising|cloning|Activating)|.*backed up)' "$LOGS/run2.log" || true)"
if [[ -z "$changes" && "$before" == "$after" ]]; then pass "run 2 changes nothing"
else
  bad "run 2 changed something"
  [[ -z "$changes" ]] || printf '%s\n' "$changes" | sed 's/^/        /'
  [[ "$before" == "$after" ]] || printf '        state fingerprint differs: %s vs %s\n' "$before" "$after"
fi
unexpected="$(grep -E '^!  ' "$LOGS/run2.log" || true)"
if [[ -z "$unexpected" ]]; then pass "run 2 prints no unexpected warnings"
else bad "run 2 warnings:"; printf '%s\n' "$unexpected" | sed 's/^/        /'; fi

# ---- 4. checks ---------------------------------------------------------------
check() { # <label> <command in the container>
  local label="$1" out
  if out="$(cx "$2" 2>&1)"; then pass "$label"
  else bad "$label"; printf '%s\n' "$out" | tail -8 | sed 's/^/        /'; fi
}

check "links point into the repo" '
  R="$HOME/dotfiles"
  for pair in .zshrc:home/.zshrc .gitconfig:home/.gitconfig .config/zsh:home/.config/zsh \
      .config/starship.toml:home/.config/starship.toml \
      ".config/Code/User/settings.json:home/Library/Application Support/Code/User/settings.json" \
      ".config/Cursor/User/keybindings.json:home/Library/Application Support/Cursor/User/keybindings.json" \
      .claude/CLAUDE.md:claude/CLAUDE.md .claude/hooks:claude/hooks; do
    p="${pair%%:*}"; want="$R/${pair#*:}"
    [ "$(readlink "$HOME/$p")" = "$want" ] && [ -e "$HOME/$p" ] || { echo "bad link: $p -> $(readlink "$HOME/$p") (want $want)"; exit 1; }
  done
  # settings.json is built (repo base + private layer), so it is a real file, not a link.
  S="$HOME/.claude/settings.json"
  [ -f "$S" ] && [ ! -L "$S" ] || { echo "settings.json: want a built file, not a link"; exit 1; }
  grep -q PreToolUse "$S" || { echo "settings.json: built file has no hooks"; exit 1; }
  [ ! -e "$HOME/Library" ] && [ ! -e "$HOME/.config/aerospace" ]'

skipped="$(grep -c 'macOS only, skipping' "$LOGS/run1.log")"
if (( skipped >= 3 )); then pass "Mac-only steps skip with a message ($skipped)"
else bad "Mac-only steps should each say 'macOS only, skipping' (found $skipped, want 3)"; fi

check "the CLI tools are on PATH" '
  export PATH="/home/linuxbrew/.linuxbrew/bin:$HOME/.linuxbrew/bin:$HOME/.local/bin:$PATH"
  miss=""; for t in git zsh python3 brew rg fd bat eza fzf zoxide starship atuin jq gh delta lazygit mise uv claude; do
    command -v "$t" >/dev/null || miss="$miss $t"; done
  [ -z "$miss" ] || { echo "missing:$miss"; exit 1; }'

check "zsh -ic exit prints nothing" '
  out="$(zsh -ic exit 2>&1 </dev/null)"; [ -z "$out" ] || { printf "%s\n" "$out" | head -10; exit 1; }'

check "bash -n on every shell script" '
  cd ~/dotfiles && rc=0
  for f in dotfiles bootstrap.sh claude/skill-sync.sh claude/statusline.sh claude/hooks/*.sh scripts/*.sh \
           scripts/install.d/*.sh scripts/git-hooks/* claude/skills/*/scripts/*.sh; do
    [ -f "$f" ] || continue
    head -1 "$f" | grep -q "^#!.*sh" && { bash -n "$f" || rc=1; }
  done; exit $rc'

check "Cursor step in a temp HOME" '
  cd ~/dotfiles; H="$(mktemp -d)"; mkdir -p "$H/.cursor" "$H/.local/bin"
  printf "#!/bin/sh\n" >"$H/.local/bin/agent"; chmod +x "$H/.local/bin/agent"
  HOME="$H" DOTFILES_STEPS=60-cursor DOTFILES_PROFILE=shared NO_COLOR=1 ./dotfiles >"$H/out.log" 2>&1 </dev/null
  ls "$H/.cursor/rules" >/dev/null 2>&1 || { tail -15 "$H/out.log"; exit 1; }'

check "Opencode config in a temp HOME" '
  cd ~/dotfiles; H="$(mktemp -d)"
  HOME="$H" DOTFILES_STEPS=57-claude-code DOTFILES_PROFILE=shared NO_COLOR=1 ./dotfiles >"$H/out.log" 2>&1 </dev/null
  for f in AGENTS.md agents themes/copper.json tui.json; do
    [ -e "$H/.config/opencode/$f" ] || { echo "missing .config/opencode/$f"; tail -15 "$H/out.log"; exit 1; }
  done
  grep -q "\"theme\": \"copper\"" "$H/.config/opencode/tui.json" || exit 1
  grep -q "generated-by: dotfiles" "$H/.config/opencode/AGENTS.md" || exit 1'

check "guard hook tests"        'cd ~/dotfiles && python3 claude/hooks/test_guard.py'
check "skill check"             'cd ~/dotfiles && python3 claude/skills/check.py'
check "agent-config tests"      'cd ~/dotfiles && python3 scripts/test_agent_config.py'
check "cursor guard tests"      'cd ~/dotfiles && python3 cursor/hooks/test_guard_cursor.py'
check "skill scripts run" '
  set -e -o pipefail
  export PATH="/home/linuxbrew/.linuxbrew/bin:$HOME/.local/bin:$PATH"
  cd ~/dotfiles
  out="$(bash "claude/skills/ps:mic-drop/scripts/mic-check.sh" --no-fetch || true)"   # exit 1 = NOT CLEAN, fine
  printf "%s" "$out" | grep -q "VERDICT:"
  bash "claude/skills/ps:handoff/scripts/handoff-path.sh" test-topic | grep -q test-topic
  python3 "claude/skills/ps:whatsleft/scripts/whatsleft-scan.py" --eta | grep -q VERDICT
  # hurry-scan exits 1 with this message when the machine has no Claude session yet
  rc=0; out="$(python3 "claude/skills/ps:whatsleft/scripts/hurry-scan.py" --brief 2>&1)" || rc=$?
  [ "$rc" -eq 0 ] || printf "%s" "$out" | grep -q "No session transcript"
  hs="claude/skills/ps:harness-optimize/scripts/harness_stats.py"; j="$(mktemp)"
  python3 "$hs" collect --out "$j" >/dev/null && python3 "$hs" inventory --in "$j" >/dev/null
  bash "claude/skills/ps:bag-mode/scripts/bag-check.sh" --eta 5m | grep -q "NOT APPLICABLE"
  echo "{}" | bash claude/statusline.sh >/dev/null'

# ---- 5. result ---------------------------------------------------------------
if (( fails == 0 )); then
  printf '\nALL PASS on %s (logs: %s)\n' "$IMAGE" "$LOGS"
  exit 0
fi
printf '\n%d check(s) failed on %s (logs: %s)\n' "$fails" "$IMAGE" "$LOGS"
exit 1
