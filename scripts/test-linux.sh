#!/usr/bin/env bash
# ============================================================================
#  scripts/test-linux.sh <image> — run ./dotfiles end to end in a fresh Linux
#  container, twice, and check the result.
#
#    bash scripts/test-linux.sh debian:13
#    bash scripts/test-linux.sh ubuntu:24.04
#    bash scripts/test-linux.sh fedora:latest     # also archlinux:latest, alpine:latest
#
#  Needs podman or docker on the host that runs it. No sudo on the host.
#  What it does:
#    1. builds a base image once (image + sudo + git + curl + a user "dev" with sudo),
#       like a fresh install that has nothing else. apt, dnf, pacman or apk.
#    2. starts a container, copies this working tree in as a git repo with no remote
#       (git-ignored files stay out), pipes a git name and email to ./dotfiles
#    3. run 1: ./dotfiles. Run 2: ./dotfiles again, which must change nothing
#    4. checks: no shell errors, the last line, run 2 time, `dots -v`, links, tools (every tool of
#       tools.list, from mise or the fallback source, and the jq link), the Rust tools,
#       `zsh -ic exit` prints nothing, bash -n, the unit tests. No Homebrew on any Linux.
#    5. removes the container (the base image stays, for speed)
#  Exit 0 only when every check passes.
#
#  Options:  --keep   leave the container running (shell in: podman exec -it NAME bash)
#            --reuse  use the kept container: sync the tree, run both runs again
#            --clean  also remove the base image at the end
#            -v       also print each PASS (the default prints failures and the last line)
#  Env:      CONTAINER_RUNTIME=podman|docker (default: the first one found)
#            TEST_LOG_DIR=<dir> for the logs (default: $TMPDIR/dots-test-linux)
#            TEST_NAME_SUFFIX=<word> added to the container name, so two runs never clash
#            TEST_PLATFORM=<os/arch> passed as --platform (default: linux/amd64 for archlinux on an
#            arm64 host, because Arch publishes no arm64 image; empty otherwise). The Arch base
#            image turns off pacman's syscall sandbox, which fails under amd64 emulation.
#  Images: Debian, Ubuntu, Fedora, Arch, Alpine (and others with one of their package managers).
# ============================================================================
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="" KEEP=0 REUSE=0 CLEAN=0 VERBOSE=""
for a in "$@"; do
  case "$a" in
    --keep) KEEP=1 ;;
    --reuse) REUSE=1; KEEP=1 ;;
    --clean) CLEAN=1 ;;
    -v|--verbose) VERBOSE=1 ;;
    -h|--help) sed -n '2,34p' "${BASH_SOURCE[0]}"; exit 0 ;;
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

PLAT=()
platform="${TEST_PLATFORM-}"
if [[ -z "${TEST_PLATFORM+x}" && "$IMAGE" == archlinux* ]]; then
  case "$(uname -m)" in arm64|aarch64) platform=linux/amd64 ;; esac
fi
[[ -n "$platform" ]] && PLAT=(--platform "$platform")

slug="$(printf '%s' "$IMAGE" | tr -c 'a-zA-Z0-9\n' '-' | sed 's/-*$//')"
# The base image: the image plus sudo, git, curl and a user "dev" with sudo. Its tag holds a
# hash of this recipe, so a changed recipe builds a new image instead of using a stale one.
RECIPE="FROM $IMAGE
ENV DEBIAN_FRONTEND=noninteractive
RUN if command -v apt-get >/dev/null; then apt-get update \\
      && apt-get install -y --no-install-recommends sudo ca-certificates curl git && rm -rf /var/lib/apt/lists/*; \\
    elif command -v dnf >/dev/null; then \\
      dnf install -y sudo ca-certificates curl git findutils tar shadow-utils procps-ng && dnf clean all; \\
    elif command -v pacman >/dev/null; then sed -i 's/^#DisableSandboxSyscalls/DisableSandboxSyscalls/' /etc/pacman.conf \\
      && pacman -Syu --noconfirm sudo ca-certificates curl git findutils tar; \\
    elif command -v apk >/dev/null; then \\
      apk add --no-cache bash sudo ca-certificates curl git findutils tar coreutils shadow; \\
    else echo \"no apt-get, dnf, pacman or apk in $IMAGE\" >&2; exit 1; fi
RUN useradd -m -s /bin/bash dev \\
 && echo 'dev ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/dev && chmod 440 /etc/sudoers.d/dev"
BASE="dots-test-base-$slug-$(printf '%s' "$RECIPE" | cksum | cut -d' ' -f1)"
NAME="dots-test-$slug${TEST_NAME_SUFFIX:+-$TEST_NAME_SUFFIX}"
LOGS="${TEST_LOG_DIR:-${TMPDIR:-/tmp}/dots-test-linux-$(id -u)}/$slug"
(umask 077; mkdir -p "$LOGS")
[[ -d "$LOGS" && ! -L "$LOGS" ]] || { printf 'bad log dir: %s\n' "$LOGS" >&2; exit 2; }
rm -f "$LOGS"/*.log

fails=0
passes=0
pass() { passes=$((passes + 1)); if [[ -n "${VERBOSE:-}" ]]; then printf 'PASS  %s\n' "$*"; fi; }
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
  if ! printf '%s\n' "$RECIPE" | "$RT" build -q ${PLAT[@]+"${PLAT[@]}"} -t "$BASE" -f - "$ctx" >"$LOGS/build.log" 2>&1
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
  "$RT" run -d ${PLAT[@]+"${PLAT[@]}"} --name "$NAME" --user dev -w /home/dev "$BASE" sleep infinity >/dev/null \
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
pack | "$RT" exec -i "$NAME" bash -lc 'tar --warning=no-unknown-keyword -xf - -C ~/dotfiles' || { bad "couldn't copy the tree"; exit 1; }
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
  { dpkg-query -W -f "\${Package}\n"; rpm -qa --qf "%{NAME}\n"; pacman -Qq; apk info; } 2>/dev/null | sort
  export PATH="/home/linuxbrew/.linuxbrew/bin:/home/linuxbrew/.linuxbrew/opt/rustup/bin:$HOME/.linuxbrew/bin:$HOME/.cargo/bin:$PATH"
  brew list --formula 2>/dev/null | sort
  rustup toolchain list 2>/dev/null; mise ls 2>/dev/null
  gh extension list 2>/dev/null; } | md5sum'

MUSL=0
cx 'compgen -G "/lib/ld-musl-*" >/dev/null' && MUSL=1
OK_WARN='^$'

# ---- 3. run 1 and run 2 ------------------------------------------------------
note "run 1 (fresh): ./dotfiles on $IMAGE ($RT, logs in $LOGS)"
t0=$SECONDS
printf 'Test User\ntest@example.com\n' | "$RT" exec -i -e NO_COLOR=1 -e DOTFILES_PROFILE=shared "$NAME" \
  bash -lc 'cd ~/dotfiles && ./dotfiles' >"$LOGS/run1.log" 2>&1
rc1=$?
note "run 1 took $((SECONDS - t0))s, exit $rc1"
if (( rc1 == 0 )); then pass "run 1 exits 0"; else bad "run 1 exit $rc1"; fi

# Warnings go to stderr as "!  text". A clean run has none.
unexpected="$(grep -E '^!  ' "$LOGS/run1.log" | grep -vE "$OK_WARN" || true)"
if [[ -z "$unexpected" ]]; then pass "run 1 prints no unexpected warnings"
else bad "run 1 warnings:"; printf '%s\n' "$unexpected" | sed 's/^/        /'; fi

before="$(cx "$STATE")"
note "run 2 (same container): ./dotfiles again"
t0=$SECONDS
"$RT" exec -i -e NO_COLOR=1 -e DOTFILES_PROFILE=shared "$NAME" bash -lc 'cd ~/dotfiles && ./dotfiles' \
  </dev/null >"$LOGS/run2.log" 2>&1
rc2=$?
t2=$((SECONDS - t0))
note "run 2 took ${t2}s, exit $rc2"
if (( rc2 == 0 )); then pass "run 2 exits 0"; else bad "run 2 exit $rc2"; fi
after="$(cx "$STATE")"
changes="$(grep -E '^(✓ (linked|removed)|»» (Installing|installing|Initialising|cloning|Activating)|.*backed up)' "$LOGS/run2.log" || true)"
if [[ -z "$changes" && "$before" == "$after" ]]; then pass "run 2 changes nothing"
else
  bad "run 2 changed something"
  [[ -z "$changes" ]] || printf '%s\n' "$changes" | sed 's/^/        /'
  [[ "$before" == "$after" ]] || printf '        state fingerprint differs: %s vs %s\n' "$before" "$after"
fi
unexpected="$(grep -E '^!  ' "$LOGS/run2.log" | grep -vE "$OK_WARN" || true)"
if [[ -z "$unexpected" ]]; then pass "run 2 prints no unexpected warnings"
else bad "run 2 warnings:"; printf '%s\n' "$unexpected" | sed 's/^/        /'; fi

# Errors that do not go through warn: a typo, a missing command, a crash.
ERR_RE='command not found|unbound variable|syntax error|bad substitution|integer expression expected'
ERR_RE+='|binary operator expected|too many arguments|No such file or directory|Permission denied'
ERR_RE+='|Traceback \(most recent call last\)|panicked at'
# The last line: the done line, with the warning count when a step warned.
want_last='^✓ dotfiles: done in [0-9hms]+ \(profile: shared\)$'
for n in 1 2; do
  errs="$(grep -E "$ERR_RE" "$LOGS/run$n.log" || true)"
  if [[ -z "$errs" ]]; then pass "run $n prints no shell or runtime errors"
  else bad "run $n errors:"; printf '%s\n' "$errs" | head -8 | sed 's/^/        /'; fi
  last="$(tail -n 1 "$LOGS/run$n.log")"
  if [[ "$last" =~ $want_last ]]; then pass "run $n ends with the done line"
  else bad "run $n last line: $last"; fi
done
if (( t2 <= 90 )); then pass "run 2 takes ${t2}s (90s at most)"; else bad "run 2 takes ${t2}s (90s at most)"; fi
if grep -q 'building the Rust' "$LOGS/run2.log"; then bad "run 2 builds the Rust tools again"
else pass "run 2 does not build the Rust tools again"; fi

# ---- 4. checks ---------------------------------------------------------------
check() { # <label> <command in the container>
  local label="$1" out
  if out="$(cx "$2" 2>&1)"; then pass "$label"
  else bad "$label"; printf '%s\n' "$out" | tail -8 | sed 's/^/        /'; fi
}

check "links point into the repo" '
  R="$HOME/dotfiles"
  for pair in .zshrc:home/.zshrc .gitconfig:home/.gitconfig .config/zsh:home/.config/zsh \
      .claude/CLAUDE.md:claude/CLAUDE.md .claude/hooks:claude/hooks; do
    p="${pair%%:*}"; want="$R/${pair#*:}"
    [ "$(readlink "$HOME/$p")" = "$want" ] && [ -e "$HOME/$p" ] || { echo "bad link: $p -> $(readlink "$HOME/$p") (want $want)"; exit 1; }
  done
  # The looks (LOOKS) are off until picked: no Starship or editor links.
  for p in .config/starship.toml .config/Code/User/settings.json .config/Cursor/User/keybindings.json; do
    [ ! -L "$HOME/$p" ] || { echo "a look is linked while LOOKS is not set: $p"; exit 1; }
  done
  # settings.json is built (repo base + private layer), so it is a real file, not a link.
  S="$HOME/.claude/settings.json"
  [ -f "$S" ] && [ ! -L "$S" ] || { echo "settings.json: want a built file, not a link"; exit 1; }
  grep -q PreToolUse "$S" || { echo "settings.json: built file has no hooks"; exit 1; }
  [ ! -e "$HOME/Library" ] && [ ! -e "$HOME/.config/aerospace" ]'

# The short output: run 2 has nothing to do, so it is one short screen with no per-file lines.
lines2="$(wc -l <"$LOGS/run2.log" | tr -d ' ')"
if (( lines2 <= 25 )) && ! grep -q 'already set up' "$LOGS/run2.log"; then pass "run 2 prints one short screen ($lines2 lines)"
else bad "run 2 prints $lines2 lines, or per-file skip lines (see $LOGS/run2.log)"; fi
if grep -q 'macOS only' "$LOGS/run1.log"; then bad "run 1 says 'macOS only' (the Mac-only steps should be silent)"
else pass "Mac-only steps are silent on Linux"; fi

# Run 3: dots -v prints everything, still exits 0, and warns about nothing new.
"$RT" exec -i -e NO_COLOR=1 -e DOTFILES_PROFILE=shared "$NAME" bash -lc 'cd ~/dotfiles && ./dotfiles -v' \
  </dev/null >"$LOGS/run3v.log" 2>&1
rc3=$?
lines3="$(wc -l <"$LOGS/run3v.log" | tr -d ' ')"
unexpected="$(grep -E '^!  ' "$LOGS/run3v.log" | grep -vE "$OK_WARN" || true)"
if (( rc3 == 0 && lines3 > lines2 )) && [[ -z "$unexpected" ]]; then pass "dots -v prints more ($lines3 lines) and exits 0"
else bad "dots -v: exit $rc3, $lines3 lines (run 2: $lines2), warnings: ${unexpected:-none}"; fi
check "no warning files left in /tmp" '! ls /tmp/dots-warn.* 2>/dev/null | grep .'

# The tool list: tools.list is the one list; no Homebrew on Linux. `scripts/tools --check` lists what is missing
# for this OS. A few tools then run, and jq (a link to jaq) reads JSON like jq.
TOOLS_PATH='export PATH="$HOME/.local/bin:$HOME/.local/share/mise/shims:$HOME/.cargo/bin:$PATH"'
check "every tool of tools.list is installed" "$TOOLS_PATH"'
  cd ~/dotfiles && bash scripts/tools --check'
check "git, zsh and the key tools start" "$TOOLS_PATH"'
  miss=""; for t in git zsh rg fd bat fzf jq gh mise uv claude; do
    command -v "$t" >/dev/null || miss="$miss $t"; done
  [ -z "$miss" ] || { echo "missing:$miss"; exit 1; }
  [ "$(echo "{\"a\":7}" | jq -r .a)" = 7 ] || { echo "jq does not read JSON"; exit 1; }
  rg --version >/dev/null && fzf --version >/dev/null && gh --version >/dev/null'
check "scripts/tools is quiet on a second run" "$TOOLS_PATH"'
  cd ~/dotfiles && out="$(bash scripts/tools 2>&1)"; case "$out" in "• Tools: "*" up to date") ;; *) printf "%s\n" "$out" | head -5; exit 1 ;; esac'

# The Rust tools: dots builds them once the toolchain is there (rustup comes from tools.list).
check "the Rust tools are built and run" '
  B=~/dotfiles/rust/agent-config/target/release
  for b in agent-config guard yt_transcribe tools; do [ -x "$B/$b" ] || { echo "missing: $B/$b"; exit 1; }; done
  [ "$(bash ~/dotfiles/claude/hooks/rust-bin yt_transcribe --print-id https://youtu.be/dQw4w9WgXcQ)" = dQw4w9WgXcQ ]'

check "zsh -ic exit prints nothing" '
  out="$(zsh -ic exit 2>&1 </dev/null)"; [ -z "$out" ] || { printf "%s\n" "$out" | head -10; exit 1; }'

check "bash -n on every shell script" '
  cd ~/dotfiles && rc=0
  for f in dotfiles bootstrap.sh claude/hooks/*.sh claude/hooks/rust-bin scripts/*.sh \
           scripts/git-hooks/* claude/skills/*/scripts/*.sh; do
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

# The Rust tests (guard, pace, agent-config, skills) run with `cargo test` on the host, not here.
check "skill check"             'cd ~/dotfiles && bash claude/hooks/rust-bin skill_check -q'
check "skill scripts run" '
  set -e -o pipefail
  export PATH="/home/linuxbrew/.linuxbrew/bin:$HOME/.local/bin:$PATH"
  cd ~/dotfiles
  rb="bash claude/hooks/rust-bin"
  out="$($rb mic_check --no-fetch || true)"   # exit 1 = NOT CLEAN, fine
  printf "%s" "$out" | grep -q "VERDICT:"
  bash "claude/skills/ps_handoff/scripts/handoff-path.sh" test-topic | grep -q test-topic
  $rb whatsleft_scan --eta | grep -q VERDICT
  # hurry_scan exits 1 with this message when the machine has no Claude session yet
  rc=0; out="$($rb hurry_scan --brief 2>&1)" || rc=$?
  [ "$rc" -eq 0 ] || printf "%s" "$out" | grep -q "No session transcript"
  j="$(mktemp)"
  $rb harness_stats collect --out "$j" >/dev/null && $rb harness_stats inventory --in "$j" >/dev/null
  $rb bag_check --eta 5m | grep -q "NOT APPLICABLE"
  echo "{}" | $rb statusline >/dev/null'

# ---- 5. result ---------------------------------------------------------------
if (( fails == 0 )); then
  printf 'ALL PASS on %s, %d checks (logs: %s)\n' "$IMAGE" "$passes" "$LOGS"
  exit 0
fi
printf '%d check(s) failed on %s (logs: %s)\n' "$fails" "$IMAGE" "$LOGS"
exit 1
