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
#    scripts/os-check.sh --which FILE...    # the OSes each file is for (the skip rule)
#
#  macOS:   scripts/test-dotfiles.sh on this Mac (temp HOME).
#  Linux:   scripts/test-linux.sh for each image in OS_CHECK_IMAGES (default
#           "debian:13 ubuntu:24.04") on the SSH host TEST_HOST (dots settings,
#           or --host), else with local podman or docker.
#  Windows: a throwaway clone of the UTM VM win11-golden (built once by
#           scripts/win-golden.sh). It starts the clone, copies the tree over SSH, runs
#           ./dotfiles twice in Git Bash, checks both exit codes, the links and
#           `pwsh -Command exit`, then deletes the clone, also after a failure.
#           The golden VM is never started.
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
    --which) shift; WHICH=("$@"); break ;;
    -h | --help) sed -n '2,25p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
  shift
done
((${#targets[@]})) || targets=(macos linux windows)
HOST="${HOST:-${TEST_HOST:-$(setting TEST_HOST)}}"
IMAGES="${OS_CHECK_IMAGES:-debian:13 ubuntu:24.04}"
RUN_ID="${OS_CHECK_ID:-os-check-$$}"
OUT="$REPO/.os-check"
mkdir -p "$OUT"

# ---- which OS a changed file is for ----------------------------------------
RE_ALL='^(dotfiles|bootstrap\.sh|scripts/(setup-lib|settings|os-check)\.sh|scripts/agent-config|rust/|claude/(settings\.json|skill-sync\.sh|hooks/))'
RE_STEP='^scripts/install\.d/'
RE_WIN='^scripts/install\.d/(15|57)-|\.psm?1$|(^|/)(windows|winget|scoop)|^home/\.config/(gh|opencode)/|^home/Library/Application Support/|^\.gitattributes$'
RE_LINUX='^(Brewfile|home/|scripts/test-linux\.sh)'
RE_LINUX_NOT='^scripts/install\.d/(15|70|72|75)-'
RE_MAC='^(Brewfile|home/|scripts/(test-dotfiles|fix-brew-permissions)\.sh)'
RE_MAC_NOT='^scripts/install\.d/(05|15|78)-'

wants() { # wants <os> <file>
  [[ "$2" =~ $RE_ALL ]] && return 0
  case "$1" in
    windows) [[ "$2" =~ $RE_WIN ]] ;;
    linux) [[ "$2" =~ $RE_LINUX ]] || { [[ "$2" =~ $RE_STEP ]] && ! [[ "$2" =~ $RE_LINUX_NOT ]]; } ;;
    macos) [[ "$2" =~ $RE_MAC ]] || { [[ "$2" =~ $RE_STEP ]] && ! [[ "$2" =~ $RE_MAC_NOT ]]; } ;;
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

# win_cleanup: stop and delete the check clone. Never touches win11-golden.
WIN_CLONE="" WIN_UTMCTL=""
win_cleanup() {
  local vm="$WIN_CLONE" i
  WIN_CLONE=""
  [[ "$vm" == win11-check-* ]] || return 0
  for i in 1 2 3; do
    "$WIN_UTMCTL" stop "$vm" --kill >/dev/null 2>&1 || true
    "$WIN_UTMCTL" delete "$vm" >/dev/null 2>&1 && return 0
    sleep 2
  done
}

run_windows() {
  is_macos || { echo "skip: the Windows check needs UTM on macOS"; return 2; }
  local u miss=() golden=win11-golden user="${WIN_USER:-dots}" key="${WIN_KEY:-$HOME/.ssh/os-check_ed25519}"
  u="$(command -v utmctl || echo /Applications/UTM.app/Contents/MacOS/utmctl)"
  if [[ ! -x "$u" ]]; then
    miss+=("Install UTM: brew install --cask utm")
  fi
  if [[ ! -x "$u" ]] || ! "$u" list 2>/dev/null | grep -qw "$golden"; then
    miss+=("Build the Windows 11 Arm64 VM $golden in UTM: bash scripts/win-golden.sh")
    miss+=("  (it needs the Windows ISO in ~/VMs/iso; the script says where to get it)")
  fi
  [[ -f "$key" ]] || miss+=("The SSH key $key is missing (scripts/win-golden.sh makes it)")
  if ((${#miss[@]})); then
    echo "FAIL  the Windows VM is not set up. Missing steps:"
    printf '  %s\n' "${miss[@]}"
    return 1
  fi
  if "$u" status "$golden" 2>/dev/null | grep -qvi stopped; then
    echo "FAIL  $golden is not shut down. os-check never starts it: shut it down in UTM"
    return 1
  fi

  WIN_UTMCTL="$u"
  trap 'win_cleanup' EXIT
  trap 'win_cleanup; exit 130' INT TERM
  # A clone left by a run that was killed hard.
  local old
  while IFS= read -r old; do WIN_CLONE="$old"; win_cleanup; done < <("$u" list | awk '$3 ~ /^win11-check-/ {print $3}')

  local vm="win11-check-$(date +%Y%m%d-%H%M%S)" ip="" end ssh_o
  ssh_o=(-i "$key" -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=no
         -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=15)
  echo "== clone $golden to $vm"
  WIN_CLONE="$vm"
  "$u" clone "$golden" --name "$vm" || { echo "FAIL  couldn't clone $golden"; return 1; }
  "$u" start "$vm" || { echo "FAIL  couldn't start $vm"; return 1; }
  echo "== wait for the guest IP and SSH (5 minutes at most)"
  end=$((SECONDS + 300))
  while ((SECONDS < end)); do
    ip="$("$u" ip-address "$vm" 2>/dev/null | grep -Eo '^[0-9]+(\.[0-9]+){3}' | head -n1 || true)"
    if [[ -n "$ip" ]] && ssh "${ssh_o[@]}" "$user@$ip" 'echo up' 2>/dev/null | grep -q up; then break; fi
    ip=""; sleep 5
  done
  [[ -n "$ip" ]] || { echo "FAIL  $vm gave no IP or SSH in 5 minutes"; return 1; }
  echo "   $vm is up at $ip"

  echo "== copy the tree to %USERPROFILE%\\dotfiles"
  pack | ssh "${ssh_o[@]}" "$user@$ip" 'rmdir /s /q "%USERPROFILE%\dotfiles" 2>nul & mkdir "%USERPROFILE%\dotfiles" & tar -xf - -C "%USERPROFILE%\dotfiles"' ||
    { echo "FAIL  couldn't copy the tree to the VM"; return 1; }

  echo "== run ./dotfiles twice in Git Bash"
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
for p in .claude/CLAUDE.md .claude/hooks .config/gh/config.yml; do
  if [ -L ~/"$p" ] && [ -e ~/"$p" ]; then echo "LINK_OK $p"; else echo "LINK_BAD $p"; fi
done
pwsh_out="$(pwsh -Command exit 2>&1)"; echo "PWSH_EXIT=$?"
[ -z "$pwsh_out" ] && echo PWSH_QUIET || { echo PWSH_NOISY; printf '%s\n' "$pwsh_out"; }
for n in 1 2; do echo "=== run$n.log"; cat ~/run$n.log; done
GUEST
  )" || true
  printf '%s\n' "$out"

  local rc=0 m
  for m in RUN1_EXIT RUN2_EXIT; do
    grep -qx "$m=0" <<<"$out" || { echo "FAIL  $m is not 0 (log lines above)"; rc=1; }
  done
  if grep -q '^LINK_BAD' <<<"$out"; then echo "FAIL  a link is missing: $(grep '^LINK_BAD' <<<"$out" | tr '\n' ' ')"; rc=1; fi
  grep -qx PWSH_QUIET <<<"$out" || { echo "FAIL  pwsh -Command exit prints an error or did not run"; rc=1; }
  ((rc)) || echo "ok: both runs exit 0, the links are there, pwsh is quiet"
  return $rc
}

# ---- run and sum up ------------------------------------------------------
fails=0 lines=()
for os in "${targets[@]}"; do
  log="$OUT/$os.log"
  if ! need "$os"; then
    lines+=("SKIP  $os: no $os change since $SINCE (--force runs it)")
    continue
  fi
  printf '»» %s check, log: .os-check/%s.log\n' "$os" "$os"
  rc=0
  "run_$os" >"$log" 2>&1 || rc=$?
  why="$(grep -m1 -E '^(FAIL|skip)' "$log" || tail -n 1 "$log" || true)"
  case $rc in
    0) lines+=("PASS  $os") ;;
    2) lines+=("SKIP  $os: ${why#skip: }") ;;
    *) lines+=("FAIL  $os: ${why#FAIL  }"); fails=$((fails + 1)) ;;
  esac
done
printf '%s\n' "${lines[@]}"
((fails)) && { printf 'RESULT: %s failed. Logs: .os-check/\n' "$fails"; exit 1; }
printf 'RESULT: pass. Logs: .os-check/\n'
