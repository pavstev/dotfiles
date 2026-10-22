#!/usr/bin/env bash
# ============================================================================
#  scripts/os-check.sh: a manual check of ./dotfiles on macOS, Linux and Windows.
#  Manual only: no git hook and no CI runs it. Run it before you commit a change
#  to setup code, or ask Claude for /os-check.
#
#    scripts/os-check.sh                    # each OS whose files changed vs HEAD
#    scripts/os-check.sh linux --force      # one OS, even with no change
#    scripts/os-check.sh --since origin/main
#    scripts/os-check.sh linux --host NAME  # Linux on that SSH host
#    scripts/os-check.sh linux --local      # Linux in local podman or docker
#    scripts/os-check.sh --which FILE...    # the OSes each file is for (the skip rule)
#
#  macOS:   scripts/test-dotfiles.sh on this Mac (temp HOME).
#  Linux:   scripts/test-linux.sh for each image in OS_CHECK_IMAGES (default
#           "debian:13 ubuntu:24.04") on the SSH host TEST_HOST (dots settings,
#           or --host), else with local podman or docker. --local (or TEST_HOST=local)
#           uses podman or docker even when TEST_HOST is set, e.g. when that host is off.
#  Windows: a throwaway clone of the UTM VM win11-golden (built once by
#           scripts/win-golden.sh). It starts the clone, copies the tree over SSH, runs
#           ./dotfiles twice in Git Bash, then a third time after a pull that brings a
#           Rust change (restart and dots.exe rebuild), checks the exit codes, the links and
#           `pwsh -Command exit`, then deletes the clone, also after a failure.
#           The golden VM is never started.
#  The OSes run at the same time. Each result line prints when its OS ends.
#  Logs go to .os-check/<os>.log (Linux: also .os-check/linux-logs/). Each run has its own
#  container names and remote folder (OS_CHECK_ID), so two checkouts can run at once.
#  Exit 0 when every OS that ran passed.
# ============================================================================
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=setup-lib.sh
. "$REPO/scripts/setup-lib.sh"

targets=() FORCE=0 SINCE=HEAD HOST="" WHICH=()
while (($#)); do
  case "$1" in
    macos | linux | windows) targets+=("$1") ;;
    all) targets=(macos linux windows) ;;
    --force) FORCE=1 ;;
    --since) SINCE="${2:?--since needs a git ref}"; shift ;;
    --host) HOST="${2:?--host needs an SSH host}"; shift ;;
    --local) HOST=local ;;
    --which) shift; WHICH=("$@"); break ;;
    -h | --help) sed -n '2,28p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
  shift
done
((${#targets[@]})) || targets=(macos linux windows)
HOST="${HOST:-${TEST_HOST:-$(setting TEST_HOST)}}"
[[ "$HOST" != local ]] || HOST="" # local: podman or docker on this machine
IMAGES="${OS_CHECK_IMAGES:-debian:13 ubuntu:24.04}"
RUN_ID="${OS_CHECK_ID:-os-check-$$}"
OUT="$REPO/.os-check"
mkdir -p "$OUT"

# ---- which OS a changed file is for ----------------------------------------
RE_ALL='^(dotfiles|bootstrap\.sh|tools\.list|scripts/(setup-lib|settings|os-check)\.sh|scripts/(agent-config|tools)|rust/|claude/(settings\.json|hooks/))'
RE_WIN='\.psm?1$|(^|/)(windows|winget|scoop)|^home/\.config/(gh|opencode)/|^home/Library/Application Support/|^\.gitattributes$'
RE_LINUX='^(home/|scripts/test-linux\.sh)'
RE_MAC='^(Brewfile|home/|scripts/(test-dotfiles|fix-brew-permissions)\.sh)'

wants() { # wants <os> <file>
  [[ "$2" =~ $RE_ALL ]] && return 0
  case "$1" in
    windows) [[ "$2" =~ $RE_WIN ]] ;;
    linux) [[ "$2" =~ $RE_LINUX ]] ;;
    macos) [[ "$2" =~ $RE_MAC ]] ;;
  esac
}

# --which FILE...: print the OSes each file is for. scripts/test-dotfiles.sh checks a table of them.
if ((${#WHICH[@]})); then
  for f in "${WHICH[@]}"; do
    oses=""
    for os in macos linux windows; do if wants "$os" "$f"; then oses="$oses $os"; fi; done
    printf '%s:%s\n' "$f" "${oses:- none}"
  done
  exit 0
fi

need() { # need <os>: true with --force, or when a staged, unstaged or new file is for <os>
  ((FORCE)) && return 0
  local f
  while IFS= read -r f; do wants "$1" "$f" && return 0; done < <(
    git -C "$REPO" diff --name-only "$SINCE" --
    git -C "$REPO" ls-files --others --exclude-standard
  )
  return 1
}

# pack: the files git would commit or list as new (ignored files and secrets stay out).
pack() {
  git -C "$REPO" ls-files -z --cached --others --exclude-standard |
    while IFS= read -r -d '' f; do [[ -e "$REPO/$f" ]] && printf '%s\0' "$f"; done |
    COPYFILE_DISABLE=1 tar -C "$REPO" --null -T - -cf -
}

# ---- one function per OS: 0 pass, 1 fail, 2 skip -------------------------
run_macos() {
  is_macos || { echo "skip: the macOS check runs on macOS only"; return 2; }
  bash "$REPO/scripts/test-dotfiles.sh"
}

run_linux() {
  local img rc=0 dir=".cache/dots-os-check/$RUN_ID" logs=".cache/dots-os-check-logs/$RUN_ID"
  rm -rf "$OUT/linux-logs"
  if [[ -z "$HOST" ]]; then
    for img in $IMAGES; do
      echo "== $img (local)"
      TEST_NAME_SUFFIX="$RUN_ID" TEST_LOG_DIR="$OUT/linux-logs" bash "$REPO/scripts/test-linux.sh" "$img" || rc=1
    done
    return $rc
  fi
  echo "== copy the tree to $HOST:~/$dir"
  if ! pack | ssh -o BatchMode=yes "$HOST" \
      "rm -rf ~/$dir && mkdir -p ~/$dir && tar --warning=no-unknown-keyword -xf - -C ~/$dir"; then
    echo "FAIL  couldn't copy the tree to $HOST"
    ssh -o BatchMode=yes "$HOST" "rm -rf ~/$dir" 2>/dev/null
    return 1
  fi
  for img in $IMAGES; do
    echo "== $img (on $HOST)"
    ssh -o BatchMode=yes "$HOST" \
      "cd ~/$dir && TEST_NAME_SUFFIX='$RUN_ID' TEST_LOG_DIR=\$HOME/$logs bash scripts/test-linux.sh '$img'" || rc=1
  done
  # Bring the logs home, then leave nothing of this run on the host.
  mkdir -p "$OUT/linux-logs"
  ssh -o BatchMode=yes "$HOST" "tar -C ~/$logs -cf - . 2>/dev/null" | tar -xf - -C "$OUT/linux-logs" 2>/dev/null
  ssh -o BatchMode=yes "$HOST" "rm -rf ~/$dir ~/$logs" 2>/dev/null
  echo "logs per image: .os-check/linux-logs/"
  return $rc
}

# UTM answers through Apple Events, and they time out. utmctl then prints "OSStatus error -1712" and
# still exits 0, or hangs for good. So every call here has a time limit and a check of its output.
WIN_CLONE="" WIN_UTMCTL="" WIN_UTM_OURS=0

# win_ctl <seconds> <utmctl args...>: prints the output. Fails on a timeout, a non-zero exit
# or an "OSStatus error" in the output. (macOS has no `timeout`; perl is always there.)
win_ctl() {
  local secs="$1" out rc=0
  shift
  out="$(perl -e 'alarm shift; exec @ARGV' "$secs" "$WIN_UTMCTL" "$@" 2>&1)" || rc=$?
  [[ -z "$out" ]] || printf '%s\n' "$out"
  ((rc == 0)) || return "$rc"
  ! grep -q 'OSStatus error' <<<"$out"
}

# win_has <name>: true when `utmctl list` answers and shows the VM. (No pipe into grep -q:
# with pipefail an early grep exit would fail the pipe.)
win_has() {
  local out
  out="$(win_ctl 20 list 2>/dev/null)" && grep -qw -- "$1" <<<"$out"
}

# win_listed <name> <seconds>: wait until `utmctl list` shows the VM.
win_listed() {
  local end=$((SECONDS + $2))
  while ((SECONDS < end)); do
    win_has "$1" && return 0
    sleep 2
  done
  return 1
}

# win_quit_utm: quit UTM, but only a UTM this run started (a hung one needs the quit).
win_quit_utm() {
  local i
  ((WIN_UTM_OURS)) || return 1
  osascript -e 'tell application "UTM" to quit' >/dev/null 2>&1 || true
  for i in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -x UTM >/dev/null || return 0
    sleep 1
  done
  pkill -x UTM 2>/dev/null || true
  sleep 1
}

# win_restart_utm <vm>: quit and reopen UTM, then wait until `utmctl list` shows the clone again.
# The fix for utmctl exit 142 (OSStatus -609, UTM's Apple Events are stuck). It never restarts a
# UTM with a running VM, and a UTM that does not answer only when this run opened it.
win_restart_utm() {
  local out
  if out="$(win_ctl 20 list 2>/dev/null)"; then
    if grep -Eiq '^[0-9A-F-]{36} +(started|starting|resuming|paused|pausing)' <<<"$out"; then
      echo "   not restarting UTM: a VM is running"
      return 1
    fi
  elif ((!WIN_UTM_OURS)); then
    return 1
  fi
  echo "   restarting UTM"
  WIN_UTM_OURS=1 # the exit trap closes the UTM opened here
  win_quit_utm
  open -a UTM || return 1
  win_listed "$1" 90
}

# win_cleanup: stop and delete the check clone. Never touches win11-golden.
win_cleanup() {
  local vm="$WIN_CLONE" i out
  WIN_CLONE=""
  [[ "$vm" == win11-check-* ]] || return 0
  for i in 1 2 3; do
    win_ctl 30 stop "$vm" --kill >/dev/null 2>&1 || true
    win_ctl 30 delete "$vm" >/dev/null 2>&1 || true
    # Trust the list, not the exit code. A list that gives no answer means UTM hangs.
    if out="$(win_ctl 20 list 2>/dev/null)"; then
      grep -qw -- "$vm" <<<"$out" || return 0
    else
      win_quit_utm || true
    fi
    sleep 2
  done
  echo "WARN  clone $vm is still there. Quit and reopen UTM, then delete it by hand."
}

# win_exit: the exit trap. Remove the clone, then close a UTM this run opened.
win_exit() {
  win_cleanup
  if ((WIN_UTM_OURS)); then win_quit_utm || true; fi
}

run_windows() {
  is_macos || { echo "skip: the Windows check needs UTM on macOS"; return 2; }
  local u miss=() golden=win11-golden user="${WIN_USER:-dots}" key="${WIN_KEY:-$HOME/.ssh/os-check_ed25519}"
  u="$(command -v utmctl || echo /Applications/UTM.app/Contents/MacOS/utmctl)"
  if [[ ! -x "$u" ]]; then
    miss+=("Install UTM: brew install --cask utm")
  else
    WIN_UTMCTL="$u"
    pgrep -x UTM >/dev/null || WIN_UTM_OURS=1
    trap 'win_exit' EXIT
    trap 'win_exit; exit 130' INT TERM
  fi
  if [[ ! -x "$u" ]] || ! win_has "$golden"; then
    miss+=("Build the Windows 11 Arm64 VM $golden in UTM: bash scripts/win-golden.sh")
    miss+=("  (it needs the Windows ISO in ~/VMs/iso; the script says where to get it)")
    miss+=("  (or UTM does not answer: quit it and run this again)")
  fi
  [[ -f "$key" ]] || miss+=("The SSH key $key is missing (scripts/win-golden.sh makes it)")
  if ((${#miss[@]})); then
    echo "FAIL  the Windows VM is not set up. Missing steps:"
    printf '  %s\n' "${miss[@]}"
    return 1
  fi
  local st rc
  st="$(win_ctl 20 status "$golden" 2>&1)" || { echo "FAIL  UTM gave no status for $golden: $st"; return 1; }
  if ! grep -qi '^stopped' <<<"$st"; then
    echo "FAIL  $golden is not shut down. os-check never starts it: shut it down in UTM"
    return 1
  fi

  # A clone left by a run that was killed hard.
  local old
  while IFS= read -r old; do WIN_CLONE="$old"; win_cleanup; done < <(win_ctl 20 list 2>/dev/null | awk '$3 ~ /^win11-check-/ {print $3}')

  local vm="win11-check-$(date +%Y%m%d-%H%M%S)" ip="" end ssh_o
  ssh_o=(-i "$key" -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=no
         -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=15)
  echo "== clone $golden to $vm"
  local try ok=0
  for try in 1 2; do
    WIN_CLONE="$vm"
    if win_ctl 120 clone "$golden" --name "$vm" && win_listed "$vm" 60; then ok=1; break; fi
    echo "   clone try $try failed"
    win_cleanup # a half-made clone goes before the next try
    sleep 5
  done
  ((ok)) || { echo "FAIL  couldn't clone $golden (OSStatus error or no clone in the list)"; return 1; }
  WIN_CLONE="$vm"
  ok=0
  for try in 1 2 3; do
    rc=0
    win_ctl 60 start "$vm" || rc=$?
    st="$(win_ctl 20 status "$vm" 2>&1)" || true
    # "started", or "starting": the IP wait below proves the boot.
    if grep -qi '^start' <<<"$st"; then ok=1; break; fi
    echo "   start try $try failed: utmctl exit $rc, status: ${st:-none}"
    # utmctl exit 142 (OSStatus -609): UTM's Apple Events are stuck. A restart of UTM fixes it.
    if ((try < 3)); then win_restart_utm "$vm" || true; fi
    sleep 5
  done
  ((ok)) || { echo "FAIL  couldn't start $vm, also after restarting UTM. With no VM running, quit and reopen UTM by hand, then run this again."; return 1; }
  echo "== wait for the guest IP and SSH (5 minutes at most)"
  end=$((SECONDS + 300))
  while ((SECONDS < end)); do
    ip="$(win_ctl 15 ip-address "$vm" 2>/dev/null | grep -Eo '^[0-9]+(\.[0-9]+){3}' | head -n1 || true)"
    if [[ -n "$ip" ]] && ssh "${ssh_o[@]}" "$user@$ip" 'echo up' 2>/dev/null | grep -q up; then break; fi
    ip=""; sleep 5
  done
  [[ -n "$ip" ]] || { echo "FAIL  $vm gave no IP or SSH in 5 minutes"; return 1; }
  echo "   $vm is up at $ip"

  echo "== copy the tree to %USERPROFILE%\\dotfiles"
  pack | ssh "${ssh_o[@]}" "$user@$ip" 'rmdir /s /q "%USERPROFILE%\dotfiles" 2>nul & mkdir "%USERPROFILE%\dotfiles" & tar -xf - -C "%USERPROFILE%\dotfiles"' ||
    { echo "FAIL  couldn't copy the tree to the VM"; return 1; }

  echo "== run ./dotfiles 3 times in Git Bash (the 3rd after a pull)"
  local out
  out="$(ssh "${ssh_o[@]}" "$user@$ip" '"C:\Program Files\Git\bin\bash.exe" -l -s' 2>&1 <<'GUEST'
cd ~/dotfiles || exit 1
git init -q -b main
git add -A
git -c core.autocrlf=false -c user.name=test -c user.email=test@example.com commit -q --allow-empty -m snapshot
rm -rf ~/origin.git
git clone -q --bare . ~/origin.git && git remote add origin ~/origin.git && git fetch -q origin && git branch -q -u origin/main
export NO_COLOR=1 DOTFILES_PROFILE=shared
printf 'Test User\ntest@example.com\n' | ./dotfiles >~/run1.log 2>&1
echo "RUN1_EXIT=$?"
./dotfiles </dev/null >~/run2.log 2>&1
echo "RUN2_EXIT=$?"
# run 3: a pull brings a Rust change, so dots restarts and rebuilds dots.exe while its copy runs.
printf '\n// os-check pull test\n' >>rust/agent-config/src/dots/mod.rs
git -c user.name=test -c user.email=test@example.com commit -qam 'pull test' && git push -q origin main && git reset -q --hard HEAD~1
./dotfiles </dev/null >~/run3.log 2>&1
echo "RUN3_EXIT=$?"
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] && echo PULL_OK || echo PULL_BAD
grep -q 'pulled the latest version' ~/run3.log && echo RESTART_OK || echo RESTART_BAD
for p in .claude/CLAUDE.md .claude/hooks .config/gh/config.yml; do
  if [ -L ~/"$p" ] && [ -e ~/"$p" ]; then echo "LINK_OK $p"; else echo "LINK_BAD $p"; fi
done
if [ ! -L ~/.gitconfig ] && git config --global --get-all include.path | grep -q gitconfig.local; then echo GITCONFIG_OK; else echo GITCONFIG_BAD; fi
pwsh_out="$(pwsh -Command exit 2>&1)"; echo "PWSH_EXIT=$?"
[ -z "$pwsh_out" ] && echo PWSH_QUIET || { echo PWSH_NOISY; printf '%s\n' "$pwsh_out"; }
# dots uninstall: the links go, the repo stays, and a second run still works.
./dotfiles uninstall -y </dev/null >~/uninstall.log 2>&1
echo "UNINSTALL_EXIT=$?"
for p in .claude/CLAUDE.md .claude/hooks .config/gh/config.yml; do
  if [ ! -e ~/"$p" ] && [ ! -L ~/"$p" ]; then echo "UNLINK_OK $p"; else echo "UNLINK_BAD $p"; fi
done
[ -f ~/dotfiles/dotfiles ] && echo REPO_KEPT || echo REPO_GONE
./dotfiles uninstall -y </dev/null >~/uninstall2.log 2>&1
echo "UNINSTALL2_EXIT=$?"
for n in 1 2 3; do echo "=== run$n.log"; cat ~/run$n.log; done
echo "=== uninstall.log"; cat ~/uninstall.log
GUEST
  )" || true
  printf '%s\n' "$out"

  local rc=0 m
  for m in RUN1_EXIT RUN2_EXIT RUN3_EXIT; do
    grep -qx "$m=0" <<<"$out" || { echo "FAIL  $m is not 0 (log lines above)"; rc=1; }
  done
  if grep -q 'uses the bash setup' <<<"$out"; then echo "FAIL  a run fell back to the bash setup (log lines above)"; rc=1; fi
  if grep -q '^LINK_BAD' <<<"$out"; then echo "FAIL  a link is missing: $(grep '^LINK_BAD' <<<"$out" | tr '\n' ' ')"; rc=1; fi
  grep -qx GITCONFIG_OK <<<"$out" || { echo "FAIL  ~/.gitconfig is a link or lacks the include of ~/.gitconfig.local"; rc=1; }
  grep -qx PWSH_QUIET <<<"$out" || { echo "FAIL  pwsh -Command exit prints an error or did not run"; rc=1; }
  grep -qx PULL_OK <<<"$out" || { echo "FAIL  run 3 did not pull the new commit"; rc=1; }
  grep -qx RESTART_OK <<<"$out" || { echo "FAIL  run 3 did not restart after the pull"; rc=1; }
  for m in UNINSTALL_EXIT UNINSTALL2_EXIT; do
    grep -qx "$m=0" <<<"$out" || { echo "FAIL  $m is not 0: dots uninstall -y failed (log lines above)"; rc=1; }
  done
  if grep -q '^UNLINK_BAD' <<<"$out"; then echo "FAIL  dots uninstall left a link: $(grep '^UNLINK_BAD' <<<"$out" | tr '\n' ' ')"; rc=1; fi
  grep -qx REPO_KEPT <<<"$out" || { echo "FAIL  dots uninstall removed the repo folder"; rc=1; }
  ((rc)) || echo "ok: the 3 runs exit 0 (the 3rd pulls, restarts and rebuilds dots.exe), the links are there, pwsh is quiet, dots uninstall removes the links and keeps the repo"
  return $rc
}

# Tests only: OS_CHECK_STUB=<file> is sourced here and may replace run_<os> (scripts/test-dotfiles.sh).
[[ -z "${OS_CHECK_STUB:-}" ]] || . "$OS_CHECK_STUB"

# ---- run and sum up ------------------------------------------------------
# Every OS runs at the same time in the background, each into its own log.

# bg_run <os>: one background job. The `||` keeps set -e off inside run_<os>, as in a plain call.
# The Windows exit trap (it deletes the clone) runs when this job exits.
bg_run() {
  local rc=0
  "run_$1" || rc=$?
  exit "$rc"
}

# result <os> <rc>: print the result line of one OS and count a failure.
result() {
  local why
  why="$(grep -m1 -E '^(FAIL|skip)' "$OUT/$1.log" || tail -n 1 "$OUT/$1.log" || true)"
  case $2 in
    0) echo "PASS  $1" ;;
    2) echo "SKIP  $1: ${why#skip: }" ;;
    *) echo "FAIL  $1: ${why#FAIL  }"; fails=$((fails + 1)) ;;
  esac
}

# stop_all: on Ctrl-C or TERM, stop the jobs and their commands. A background job ignores Ctrl-C,
# so it gets TERM. The Windows job then deletes its clone, and this waits for that.
stop_all() {
  local p
  for p in ${pids[@]+"${pids[@]}"}; do
    [[ -n "$p" ]] || continue
    pkill -TERM -P "$p" 2>/dev/null || true
    kill -TERM "$p" 2>/dev/null || true
  done
  wait
}

fails=0 oses=() pids=()
trap 'stop_all; exit 130' INT TERM
for os in "${targets[@]}"; do
  if ! need "$os"; then
    echo "SKIP  $os: no $os change since $SINCE (--force runs it)"
    continue
  fi
  printf '»» %s check, log: .os-check/%s.log\n' "$os" "$os"
  bg_run "$os" >"$OUT/$os.log" 2>&1 &
  oses+=("$os") pids+=("$!")
done

# Print each result as soon as its job ends (bash 3.2 on macOS has no `wait -n`).
left=${#pids[@]}
while ((left)); do
  for i in "${!pids[@]}"; do
    p="${pids[$i]}"
    [[ -n "$p" ]] || continue
    kill -0 "$p" 2>/dev/null && continue
    rc=0
    wait "$p" || rc=$?
    result "${oses[$i]}" "$rc"
    pids[$i]="" left=$((left - 1))
  done
  if ((left)); then sleep 1; fi
done
trap - INT TERM
((fails)) && { printf 'RESULT: %s failed. Logs: .os-check/\n' "$fails"; exit 1; }
printf 'RESULT: pass. Logs: .os-check/\n'
