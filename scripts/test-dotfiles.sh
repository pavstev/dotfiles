#!/usr/bin/env bash
# ============================================================================
#  scripts/test-dotfiles.sh — run ./dotfiles in a temp HOME and check what it
#  links and saves. Mac and Linux, no container, about 10 seconds.
#
#  It copies the tracked files of this working tree (your edits included) into
#  a temp repo with a local "origin", then runs ./dotfiles with no install steps
#  (DOTFILES_STEPS=none) and no terminal, so nothing installs or asks.
#  Checks: first run, the owner profile saved, the owner rules linked and
#  unlinked, a run that changes nothing, and the restart after a pull.
#  It builds the Rust `dots` and `tools` once here (cargo must work) and runs ./dotfiles and
#  scripts/tools with them.
#  The Windows part of dots is checked by scripts/os-check.sh windows.
#
#  bash scripts/test-dotfiles.sh          # --keep leaves the temp folder
#  Prints only failures, then one closing line. -v also prints each PASS.
# ============================================================================
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEEP="" VERBOSE=""
for a in "$@"; do
  case "$a" in
    --keep) KEEP=1 ;;
    -v|--verbose) VERBOSE=1 ;;
    *) printf 'usage: test-dotfiles.sh [--keep] [-v]\n' >&2; exit 2 ;;
  esac
done

# The Rust dots, built once. DOTS_TEST_BIN names it for the runs below.
if [[ -z "${DOTS_TEST_BIN:-}" ]]; then
  L="$(mktemp -d)"
  if RUSTUP_AUTO_INSTALL=0 cargo build --release --quiet --locked --bin dots --bin tools \
      --manifest-path "$REPO/rust/agent-config/Cargo.toml" >"$L/build.log" 2>&1; then
    rm -rf "$L"
    DOTS_TEST_BIN="$REPO/rust/agent-config/target/release/dots" exec bash "$0" "$@"
  fi
  printf 'FAIL  no Rust build (log: %s)\n' "$L/build.log"
  exit 1
fi

T="$(mktemp -d)"
T="$(cd "$T" && pwd -P)"
cleanup() { if [[ -n "$KEEP" ]]; then printf 'kept: %s\n' "$T"; else rm -rf "$T"; fi; }
trap cleanup EXIT

fails=0 passes=0
pass() { passes=$((passes + 1)); if [[ -n "$VERBOSE" ]]; then printf 'PASS  %s\n' "$*"; fi; }
bad()  { printf 'FAIL  %s\n' "$*"; fails=$((fails + 1)); }

# git for the setup only: no user config, no hooks, no signing. A clone copies the objects: a
# hard-linked local clone in the temp folder failed about 1 run in 8 on macOS.
tgit() {
  local a=("$@")
  if [[ "$1" == clone ]]; then a=(clone --no-hardlinks "${@:2}"); fi
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git -c user.name=Test -c user.email=test@example.com \
    -c core.hooksPath=/dev/null -c commit.gpgsign=false -c init.defaultBranch=main "${a[@]}"
}

# 1. The tracked files of this working tree, as one commit, with a bare origin.
mkdir -p "$T/src" "$T/home"
(cd "$REPO" && git ls-files -z | while IFS= read -r -d '' f; do [[ -e "$f" || -L "$f" ]] && printf '%s\0' "$f"; done |
  xargs -0 tar -cf -) | tar -xf - -C "$T/src" || { bad "couldn't copy the tree"; exit 1; }
tgit -C "$T/src" init -q && tgit -C "$T/src" add -A && tgit -C "$T/src" commit -qm base || { bad "temp repo"; exit 1; }
tgit clone -q --bare "$T/src" "$T/origin.git" && tgit clone -q "$T/origin.git" "$T/repo" || { bad "temp origin"; exit 1; }
R="$T/repo"
H="$T/home"
SETTINGS="$H/.config/dotfiles/settings"
RULE="$H/.claude/rules/owner.md"
LIVE="$H/.claude/settings.json"
LAYER="$H/.config/dotfiles/claude-settings.json"
OWNER="$H/.config/dotfiles/owner.md"
OC="$H/.config/opencode"

# run <log> [env...]: ./dotfiles in the temp HOME, no terminal, no steps.
run() {
  local log="$1"; shift
  env HOME="$H" NO_COLOR=1 DOTFILES_STEPS=none DOTS_BIN="$DOTS_TEST_BIN" "$@" \
    bash "$R/dotfiles" >"$T/$log" 2>&1 <"$T/stdin"
}
# fingerprint: every link under the temp HOME and the saved settings.
fingerprint() {
  { find "$H" -type l 2>/dev/null | sort | while IFS= read -r l; do printf '%s -> %s\n' "$l" "$(readlink "$l")"; done
    cat "$SETTINGS" "$H/.gitconfig.local" "$LIVE" "$LAYER" \
        "$OC/AGENTS.md" "$OC/tui.json" "$OC/themes/copper.json" "$OC/.dotfiles-managed.json" 2>/dev/null
  } | cksum
}

# 2. First run: name and email come from stdin.
printf 'Test User\ntest@example.com\n' >"$T/stdin"
if run run1.log; then pass "first run exits 0"; else bad "first run exit $? (log: run1.log)"; fi
grep -q '^GIT_NAME="Test User"' "$SETTINGS" 2>/dev/null && pass "git name saved" || bad "git name not saved"
[[ "$(readlink "$H/.claude/CLAUDE.md")" == "$R/claude/CLAUDE.md" ]] && pass "CLAUDE.md linked" || bad "CLAUDE.md not linked"
[[ "$(readlink "$H/.zshrc")" == "$R/home/.zshrc" ]] && pass ".zshrc linked" || bad ".zshrc not linked"
grep -q '^PROFILE="[a-z]' "$SETTINGS" 2>/dev/null && bad "PROFILE saved without private rules" || pass "no private rules: PROFILE stays empty"
[[ ! -e "$RULE" && ! -L "$RULE" ]] && pass "no owner rules link" || bad "owner rules linked on the shared profile"
grep -q 'profile: shared' "$T/run1.log" && pass "profile is shared" || bad "profile not shared"
# Claude settings: a built file, the repo base only while there is no private layer.
[[ -f "$LIVE" && ! -L "$LIVE" ]] && pass "settings.json is a built file" || bad "settings.json is a link or missing"
[[ ! -e "$LAYER" ]] && pass "no private layer yet" || bad "a private layer appeared on its own"
# Presets: the shared profile links all but ship-to-main and merges their fragments over the base.
PRE="$H/.claude/rules"
[[ ! -e "$PRE/preset-ship-to-main.md" ]] && pass "ship-to-main stays off on shared" || bad "ship-to-main linked on shared"
[[ "$(readlink "$PRE/preset-workflow.md")" == "$R/claude/presets/workflow/rules.md" ]] && pass "a preset links into the repo" || bad "preset-workflow.md not linked into the repo"
grep -q '"outputStyle"' "$LIVE" && pass "preset fragment merged into settings.json" || bad "preset fragment missing from settings.json"
! grep -q '"outputStyle"' "$R/claude/settings.json" && pass "the repo base holds no preset keys" || bad "the repo base holds a preset key"
wf_line="$(grep -m1 '^- ' "$R/claude/presets/workflow/rules.md")"
grep -qF -- "$wf_line" "$OC/AGENTS.md" 2>/dev/null && pass "presets reach Opencode AGENTS.md" || bad "presets missing from Opencode AGENTS.md"
# Opencode: the rules, the agents and the copper theme are generated on every run.
grep -q 'generated-by: dotfiles' "$OC/AGENTS.md" 2>/dev/null && pass "Opencode AGENTS.md generated" || bad "Opencode AGENTS.md missing"
[ -d "$OC/agents" ] && pass "Opencode agents dir" || bad "Opencode agents dir missing"
[ -f "$OC/themes/copper.json" ] && pass "Opencode copper theme" || bad "Opencode copper theme missing"
grep -q '"theme": "copper"' "$OC/tui.json" 2>/dev/null && pass "tui.json names the copper theme" || bad "tui.json does not name the copper theme"

# 3. Private rules appear: the owner profile is saved and the rules link.
: >"$T/stdin"
printf '# Owner rules\n' >"$OWNER"
if run run2.log; then pass "owner run exits 0"; else bad "owner run exit $?"; fi
grep -q '^PROFILE="owner"' "$SETTINGS" && pass "owner profile saved" || bad "owner profile not saved"
[[ "$(readlink "$RULE")" == "$OWNER" ]] && pass "owner rules linked" || bad "owner rules not linked"
grep -q 'Owner rules' "$OC/AGENTS.md" 2>/dev/null && pass "owner rules reach Opencode" || bad "owner rules missing from Opencode AGENTS.md"
[[ "$(readlink "$PRE/preset-ship-to-main.md")" == "$R/claude/presets/ship-to-main/rules.md" ]] && pass "ship-to-main linked on owner" || bad "ship-to-main not linked on owner"

# 4. Same again: nothing changes.
before="$(fingerprint)"
run run3.log
after="$(fingerprint)"
[[ "$before" == "$after" ]] && pass "second run changes nothing" || bad "second run changed links or settings"
grep -qE 'linked |backed up' "$T/run3.log" && bad "second run linked or backed up something" || pass "second run links nothing new"
# Output: a run with nothing to do is one short screen, and -v / DOTFILES_VERBOSE=1 prints it all.
[[ "$(wc -l <"$T/run3.log")" -le 20 ]] && pass "second run prints one short screen" || bad "second run prints $(wc -l <"$T/run3.log") lines"
grep -q 'already set up' "$T/run3.log" && bad "second run prints a skip line per file" || pass "second run prints no per-file skip lines"
run run3v.log DOTFILES_VERBOSE=1
grep -q 'already set up' "$T/run3v.log" && pass "DOTFILES_VERBOSE=1 prints the skip lines" || bad "DOTFILES_VERBOSE=1 printed no skip lines"

# 4b. Claude changes its settings: the change is kept in the private layer.
live_json="$(cat "$LIVE")"
printf '{\n  "testKeptKey": true,%s\n' "${live_json#\{}" >"$LIVE"
run run3b.log
grep -q 'kept in your private layer: testKeptKey' "$T/run3b.log" && pass "Claude's change reported" || bad "Claude's change not reported"
grep -q '"testKeptKey": true' "$LAYER" 2>/dev/null && pass "Claude's change kept in the private layer" || bad "Claude's change lost"
grep -q '"testKeptKey": true' "$LIVE" && pass "built file still has it" || bad "built file lost it"
[[ -z "$(git -C "$R" status --porcelain)" ]] && pass "repo stays clean" || bad "repo has changes"
run run3c.log
grep -q 'kept' "$T/run3c.log" && bad "kept again on the next run" || pass "next run keeps nothing new"

# 5. Back to shared: the owner rules link goes, the rules file stays.
printf 'PROFILE="shared"\n' >>"$SETTINGS"
run run4.log
[[ ! -e "$RULE" && ! -L "$RULE" ]] && pass "owner rules unlinked" || bad "owner rules link still there"
grep -q 'removed the owner rules link' "$T/run4.log" && pass "unlink reported" || bad "unlink not reported"
[[ -f "$OWNER" ]] && pass "private rules file kept" || bad "private rules file gone"
[[ ! -e "$PRE/preset-ship-to-main.md" && ! -L "$PRE/preset-ship-to-main.md" ]] && pass "ship-to-main link removed on shared" || bad "ship-to-main link still there on shared"
# A preset set off loses its link; a regular file named like a preset link is never touched.
printf 'PRESETS="workflow"\n' >>"$SETTINGS"
printf 'mine\n' >"$PRE/preset-extra.md"
run run4b.log
[[ "$(find "$PRE" -name 'preset-*.md' -type l | wc -l | tr -d ' ')" == 1 ]] && pass "PRESETS list keeps one link" || bad "PRESETS list did not leave one link"
[[ "$(cat "$PRE/preset-extra.md")" == mine ]] && pass "a regular preset-*.md file is untouched" || bad "a regular preset-*.md file was changed"
printf 'PRESETS=""\n' >>"$SETTINGS"
run run4c.log

# 5b. Looks: off until picked. On links Starship; off removes the link and brings back the
#     file it replaced.
SS="$H/.config/starship.toml"
[[ ! -L "$SS" ]] && pass "no Starship link while LOOKS is not set" || bad "Starship linked while LOOKS is not set"
printf 'my prompt\n' >"$SS"
printf 'LOOKS="starship"\n' >>"$SETTINGS"
run run4d.log
[[ "$(readlink "$SS")" == "$R/home/.config/starship.toml" ]] && pass "LOOKS=starship links the prompt" || bad "LOOKS=starship did not link the prompt"
printf 'LOOKS="none"\n' >>"$SETTINGS"
run run4e.log
[[ ! -L "$SS" && "$(cat "$SS" 2>/dev/null)" == "my prompt" ]] && pass "LOOKS=none brings back the old file" || bad "LOOKS=none did not restore the old file"
rm -f "$SS"
printf 'LOOKS=""\n' >>"$SETTINGS"

# 6. A new commit on origin: the run pulls it and restarts with the new script.
tgit clone -q "$T/origin.git" "$T/other" &&
  sed 's/^set -u$/set -u; echo "dotfiles: the new shim ran" >\&2/' "$T/other/dotfiles" >"$T/dotfiles.new" &&
  cat "$T/dotfiles.new" >"$T/other/dotfiles" &&
  tgit -C "$T/other" commit -qam "new script" && tgit -C "$T/other" push -q origin HEAD:main ||
  bad "couldn't push a new commit to the temp origin"
run run5.log
grep -q 'pulled the latest version' "$T/run5.log" && pass "pull restarts the script" || bad "no restart after the pull"
grep -q 'dotfiles: the new shim ran' "$T/run5.log" && pass "the new script ran" || bad "the old script ran after the pull"

# 7. Windows safety: no tracked path may hold a name Windows cannot check out.
. "$REPO/scripts/git-hooks/windows-paths.sh"
winbad="$(cd "$REPO" && git ls-files | windows_bad_paths)"
[[ -z "$winbad" ]] && pass "every tracked path is Windows-safe" || { bad "paths Windows cannot hold: $(printf '%s' "$winbad" | head -3 | tr '\n' ' ')"; }

# 8. bootstrap.sh: a fresh clone, and the repair of a clone with no checkout (what
#    a failed Windows clone leaves behind).
H2="$T/home2"; mkdir -p "$H2"
printf 'Test User\ntest@example.com\n' >"$T/stdin"
bs() { # bs <log> <dir>
  env HOME="$H2" NO_COLOR=1 DOTFILES_STEPS=none GIT_CONFIG_GLOBAL=/dev/null DOTFILES_REPO="file://$T/origin.git" DOTFILES="$2" \
    DOTS_BIN="$DOTS_TEST_BIN" bash "$REPO/bootstrap.sh" >"$T/$1" 2>&1 <"$T/stdin"
}
if bs bs1.log "$T/bs1" && [[ -f "$T/bs1/dotfiles" ]]; then pass "bootstrap: fresh clone runs"; else bad "bootstrap: fresh clone (log: bs1.log)"; fi
[[ "$(git -C "$T/bs1" config core.autocrlf)" == false ]] && pass "bootstrap: autocrlf off in the clone" || bad "bootstrap: autocrlf not off"
tgit clone -q --no-checkout "$T/origin.git" "$T/bs2" && [[ ! -f "$T/bs2/dotfiles" ]] || bad "couldn't make a clone without a checkout"
if bs bs2.log "$T/bs2" && [[ -f "$T/bs2/dotfiles" ]]; then pass "bootstrap: a clone with no checkout is repaired"; else bad "bootstrap: repair (log: bs2.log)"; fi
grep -q 'Repairing' "$T/bs2.log" && pass "bootstrap: says it repairs" || bad "bootstrap: repair not reported"
rc=0; DOTFILES_REPO="$T/missing.git" DOTFILES="$T/bs3" HOME="$H2" GIT_CONFIG_GLOBAL=/dev/null bash "$REPO/bootstrap.sh" >"$T/bs3.log" 2>&1 <"$T/stdin" || rc=$?
[[ "$rc" -ne 0 && ! -e "$T/bs3" ]] && pass "bootstrap: a failed clone leaves nothing behind" || bad "bootstrap: failed clone left $T/bs3 or exited 0"

# 10. The last line: the done line, or the warning count of every step. A test step in the
#     pull from a missing origin warns once; the count file is gone after the run.
mkdir -p "$T/tmp"
[[ "$(tail -n 1 "$T/run3.log")" =~ ^✓\ dotfiles:\ done\ in\ [0-9hms]+\ \(profile:\ [a-z]+\)$ ]] && pass "a clean run ends with the done line" \
  || bad "clean run last line: $(tail -n 1 "$T/run3.log")"
git -C "$R" remote set-url origin "$T/no-such-origin.git"     # the pull fails: one warning
run run10.log TMPDIR="$T/tmp"
git -C "$R" remote set-url origin "$T/origin.git"
grep -q '^!  dotfiles: done in [0-9hms]* with 1 warning (profile: ' "$T/run10.log" && pass "the last line counts the warnings of a run" \
  || bad "warning count line missing (log: run10.log)"
[[ -z "$(ls "$T/tmp" | grep dots-warn)" ]] && pass "no warning count file left" || bad "warning count file left in TMPDIR"

# 11. Linux package managers, faked: a fake `uname` says Linux, a fake package manager and
#     its query tool say every package but zsh is there, and a fake sudo runs the command.
#     PATH holds only the fakes and a few real tools, so the host's own package manager stays out.
fake_linux() { # fake_linux <dir> <manager or none>
  local d="$1" m="$2" t
  mkdir -p "$d"
  for t in bash mktemp id tr tail rm grep cat env; do ln -sf "$(command -v "$t")" "$d/$t"; done
  printf '#!/bin/sh\ncase "$1" in -s) echo Linux ;; *) echo x86_64 ;; esac\n' >"$d/uname"
  printf '#!/bin/sh\n"$@"\n' >"$d/sudo"
  # each fake logs its arguments; a query for zsh fails, every other package is there
  case "$m" in
    apt-get) printf '#!/bin/sh\necho "apt-get $*" >>"%s/log"\n' "$d" >"$d/apt-get"
      printf '#!/bin/sh\nfor a; do p="$a"; done\n[ "$p" = zsh ] || echo "install ok installed"\n' >"$d/dpkg-query" ;;
    dnf) printf '#!/bin/sh\necho "dnf $*" >>"%s/log"\n' "$d" >"$d/dnf"
      printf '#!/bin/sh\n[ "$2" != zsh ]\n' >"$d/rpm" ;;
    pacman) printf '#!/bin/sh\nif [ "$1" = -Q ]; then [ "$2" != zsh ]; else echo "pacman $*" >>"%s/log"; fi\n' "$d" >"$d/pacman" ;;
    apk) printf '#!/bin/sh\nif [ "$1" = info ]; then [ "$3" != zsh ]; else echo "apk $*" >>"%s/log"; fi\n' "$d" >"$d/apk" ;;
  esac
  chmod +x "$d"/* 2>/dev/null
}
lx() { # lx <dir>: source setup-lib.sh as on that fake Linux, print the manager, list, missing, rc
  env -i HOME="$H" PATH="$1" NO_COLOR=1 "$1/bash" -c '. "$0/scripts/setup-lib.sh"
    printf "%s|%s|%s|" "$LINUX_PKG" "${LINUX_PREREQS[*]}" "$(linux_prereqs_missing | tr "\n" " ")"
    linux_prereqs >/dev/null 2>&1; echo "rc=$?"' "$R"
}
for m in apt-get dnf pacman apk; do
  d="$T/linux-$m"; fake_linux "$d" "$m"; out="$(lx "$d")"
  case "$m" in
    apt-get) want_list="build-essential ca-certificates curl file git procps zsh"
      want_log="apt-get install -y -qq --no-install-recommends zsh" ;;
    dnf) want_list="gcc gcc-c++ make ca-certificates curl file git procps-ng zsh"; want_log="dnf install -y -q zsh" ;;
    pacman) want_list="base-devel ca-certificates curl file git procps-ng zsh"
      want_log="pacman -S --needed --noconfirm zsh" ;;
    apk) want_list="bash build-base ca-certificates curl file git zsh"; want_log="apk add -q --no-cache zsh" ;;
  esac
  [[ "$out" == "$m|$want_list|zsh |rc=0" ]] && pass "Linux $m: finds the manager, its list and the missing zsh" \
    || bad "Linux $m: got '$out'"
  grep -qxF "$want_log" "$d/log" 2>/dev/null && pass "Linux $m: installs zsh with the right command" \
    || bad "Linux $m: install command: $(cat "$d/log" 2>/dev/null | tr '\n' ';')"
done
d="$T/linux-none"; fake_linux "$d" none; out="$(lx "$d")"
[[ "$out" == "|cc make curl file git ps zsh|cc make curl file git ps zsh |rc=1" ]] \
  && pass "Linux with no known manager: lists the missing commands and fails" || bad "Linux with no manager: got '$out'"

# 12. The os-check skip rule: which OS each changed file is for.
which_want='dotfiles: macos linux windows
scripts/setup-lib.sh: macos linux windows
rust/agent-config/src/lib.rs: macos linux windows
claude/hooks/rust-bin: macos linux windows
tools.list: macos linux windows
scripts/tools: macos linux windows
Brewfile: macos
home/.zshrc: macos linux
home/.config/gh/config.yml: macos linux windows
.gitattributes: windows
scripts/test-linux.sh: linux
scripts/test-dotfiles.sh: macos
README.md: none
claude/skills/ps_handoff/SKILL.md: none'
which_got="$(bash "$REPO/scripts/os-check.sh" --which $(printf '%s\n' "$which_want" | cut -d: -f1))"
[[ "$which_got" == "$which_want" ]] && pass "os-check maps each file to the right OS" \
  || bad "os-check map differs: $(diff <(printf '%s\n' "$which_want") <(printf '%s\n' "$which_got") | grep '^[<>]' | head -3 | tr '\n' ' ')"

# 12b. The os-check run, in the clean temp tree with fake OS checks (OS_CHECK_STUB). No change
#      skips every OS. With --force the 3 run at the same time, so the fastest result prints first.
out="$(bash "$T/src/scripts/os-check.sh" 2>&1)"; rc=$?
want='SKIP  macos: no macos change since HEAD (--force runs it)
SKIP  linux: no linux change since HEAD (--force runs it)
SKIP  windows: no windows change since HEAD (--force runs it)
RESULT: pass. Logs: .os-check/'
[[ "$out|$rc" == "$want|0" ]] && pass "os-check skips every OS with no change" || bad "os-check skip run: rc=$rc, got '$out'"
cat >"$T/os-stub.sh" <<'EOF'
run_macos() { sleep 3; echo ok; }
run_linux() { echo "FAIL  boom"; return 1; }
run_windows() { sleep 1; echo "skip: no UTM here"; return 2; }
EOF
out="$(OS_CHECK_STUB="$T/os-stub.sh" bash "$T/src/scripts/os-check.sh" --force 2>&1)"; rc=$?
want='»» macos check, log: .os-check/macos.log
»» linux check, log: .os-check/linux.log
»» windows check, log: .os-check/windows.log
FAIL  linux: boom
SKIP  windows: no UTM here
PASS  macos
RESULT: 1 failed. Logs: .os-check/'
[[ "$out|$rc" == "$want|1" ]] && pass "os-check runs the OSes at once and prints each result as it ends" \
  || bad "os-check --force with fake checks: rc=$rc, got '$out'"

# 12c. --local and TEST_HOST=local run Linux in local containers even when TEST_HOST names a host.
echo 'run_linux() { echo "host=[$HOST]"; }' >"$T/os-host.sh"
host_got=""
for a in "far|" "far|--local" "local|"; do
  OS_CHECK_STUB="$T/os-host.sh" TEST_HOST="${a%%|*}" bash "$T/src/scripts/os-check.sh" linux --force ${a#*|} >/dev/null 2>&1
  host_got+="$(cat "$T/src/.os-check/linux.log" 2>/dev/null) "
done
[[ "$host_got" == "host=[far] host=[] host=[] " ]] && pass "os-check --local and TEST_HOST=local skip the SSH host" \
  || bad "os-check host pick: got '$host_got'"

# 13. The tool list: scripts/tools (a shim over the Rust `tools` bin) with a fake mise. No network,
#     nothing installs for real. (cargo test covers the Rust resolver; this covers the shim:
#     arguments, TOOLS_LIST, the exit codes, and the one copy of the install-Rust code.)
TB="${DOTS_TEST_BIN%/*}/tools"
[[ -x "$TB" ]] || bad "no tools bin at $TB"
FB="$T/toolbin"; mkdir -p "$FB"
for t in bash env sh cat grep tr id sed uname dirname mktemp rm mkdir chmod date sleep tail head awk cygpath; do
  p="$(command -v "$t" 2>/dev/null)" && ln -sf "$p" "$FB/$t"
done
cat >"$FB/mise" <<'EOS'
#!/bin/sh
# fake mise: `mise --yes use -g id@latest ...` logs the call and makes a command per id (its last path part)
echo "mise $*" >>"$FAKE_LOG"
[ "$1" = --yes ] && shift
[ "$1" = use ] || exit 0
shift; [ "$1" = -g ] && shift
for a; do
  case "$a" in bad@latest|*/bad@latest) exit 1 ;; esac
  n="${a##*/}"; n="${n%@latest}"; printf '#!/bin/sh\nexit 0\n' >"$FAKE_BIN/$n"; chmod +x "$FAKE_BIN/$n"
done
EOS
chmod +x "$FB/mise"
printf 'a | tool-a | all | mise:x/tool-a\nb | tool-b | all | mise:x/tool-b\nu | tool-u | unix | mise:x/tool-u\nw | tool-w | windows | mise:x/tool-w\n' >"$T/tools.t.list"
H4="$T/home4"; mkdir -p "$H4"
tools() { env -i HOME="$H4" PATH="$FB" NO_COLOR=1 TOOLS_BIN="$TB" TOOLS_LIST="$T/tools.t.list" FAKE_LOG="$T/mise.log" FAKE_BIN="$FB" \
  "$FB/bash" "$R/scripts/tools" "$@"; }
rm -f "$T/mise.log"
out="$(tools --check 2>&1)"; rc=$?
[[ "$rc" -eq 1 && "$out" == "missing (3): a b u" ]] && pass "tools --check lists what is missing for this OS" || bad "tools --check: rc=$rc '$out'"
out="$(tools 2>&1)"; rc=$?
[[ "$rc" -eq 0 && "$(grep -c '^mise --yes use -g' "$T/mise.log")" -eq 3 ]] && pass "tools installs each missing tool through mise" || bad "tools: rc=$rc mise calls: $(cat "$T/mise.log" 2>/dev/null | tr '\n' ';')"
grep -q 'use -g x/tool-a@latest' "$T/mise.log" && grep -q 'use -g x/tool-b@latest' "$T/mise.log" && grep -q 'use -g x/tool-u@latest' "$T/mise.log" && ! grep -q 'tool-w' "$T/mise.log" \
  && pass "tools sends only this OS's tools (a Windows-only tool is left out)" || bad "tools: wrong ids: $(cat "$T/mise.log" | tr '\n' ';')"
grep -q 'Tools: 3 installed' <<<"$out" && pass "tools prints the count of what it installed" || bad "tools: no count line: $out"
out="$(tools 2>&1)"
[[ "$out" == *"Tools: 3 up to date"* && "$(grep -c 'mise --yes' "$T/mise.log")" -eq 3 ]] && pass "a second run installs nothing and prints one count line" || bad "tools run 2: '$out'"
out="$(tools -q 2>&1)"; [[ -z "$out" ]] && pass "tools -q prints nothing when nothing changed" || bad "tools -q: '$out'"
printf 'bad | tool-bad | all | mise:x/bad\ng | tool-g | all | mise:x/tool-g\n' >"$T/tools.t2.list"
out="$(env -i HOME="$H4" PATH="$FB" NO_COLOR=1 TOOLS_BIN="$TB" TOOLS_LIST="$T/tools.t2.list" FAKE_LOG="$T/mise.log" FAKE_BIN="$FB" "$FB/bash" "$R/scripts/tools" 2>&1)"; rc=$?
[[ "$rc" -eq 0 ]] && grep -q 'bad did not install' <<<"$out" && [[ -x "$FB/tool-g" ]] && pass "one bad tool is a warning and the others still install" || bad "tools with a bad tool: rc=$rc '$out'"
out="$(env -i HOME="$H4" PATH="$FB" TOOLS_BIN="$FB/no-such-bin" "$FB/bash" "$R/scripts/tools" --list 2>&1)"; rc=$?
[[ "$rc" -eq 1 && "$out" == *"TOOLS_BIN is not a program"* ]] && pass "tools says so when TOOLS_BIN is not a program" || bad "tools with a bad TOOLS_BIN: rc=$rc '$out'"
out="$(env -i HOME="$H4" PATH="$FB" "$FB/bash" "$R/scripts/tools" --help 2>&1)"; rc=$?
[[ "$rc" -eq 0 && "$out" == *"scripts/tools --check"* ]] && pass "tools --help needs no Rust" || bad "tools --help: rc=$rc '$out'"
# The one list: no formula in the Brewfile that tools.list also holds.
dup=""; while read -r f; do grep -qE "\|[[:space:]]*$f[[:space:]]*\|" "$R/tools.list" && dup="$dup $f"; done < <(sed -nE 's/^brew "([^"]+)".*/\1/p' "$R/Brewfile")
[[ -z "$dup" ]] && pass "no tool is in both the Brewfile and tools.list" || bad "in both lists:$dup"

# 13b. `scripts/agent-config --bins` retries a bin that is missing or stale, also when agent-config
#      itself is fresh (after a partial build failure only agent-config is built). A fake crate with
#      two bins and a fake cargo that logs its calls and fails: no build here, nothing installs.
FR="$T/binsrepo"; mkdir -p "$FR/scripts" "$FR/rust/agent-config/src" "$FR/rust/agent-config/target/release" "$T/fakecargo"
cp "$R/scripts/agent-config" "$R/scripts/setup-lib.sh" "$R/scripts/linux-prereqs.txt" "$FR/scripts/"
printf '[package]\nname = "x"\n\n[[bin]]\nname = "agent-config"\npath = "src/main.rs"\n\n[[bin]]\nname = "beta"\npath = "src/b.rs"\n\n[[bench]]\nname = "manual"\n' >"$FR/rust/agent-config/Cargo.toml"
printf 'fn main() {}\n' >"$FR/rust/agent-config/src/main.rs"; cp "$FR/rust/agent-config/src/main.rs" "$FR/rust/agent-config/src/b.rs"
touch -t 202001010000 "$FR/rust/agent-config/Cargo.toml" "$FR/rust/agent-config/src/main.rs" "$FR/rust/agent-config/src/b.rs"
printf '#!/bin/sh\necho "cargo $*" >>"$FAKE_LOG"\nexit 1\n' >"$T/fakecargo/cargo"; chmod +x "$T/fakecargo/cargo"
printf '#!/bin/sh\nexit 0\n' >"$FR/rust/agent-config/target/release/agent-config"; chmod +x "$FR/rust/agent-config/target/release/agent-config"
bins() { env HOME="$H4" PATH="$T/fakecargo:$PATH" NO_COLOR=1 FAKE_LOG="$T/cargo.log" bash "$FR/scripts/agent-config" --bins >/dev/null 2>&1; }
rm -f "$T/cargo.log"; bins
[[ "$(grep -c '^cargo build' "$T/cargo.log" 2>/dev/null)" -ge 1 ]] && pass "--bins builds again when a bin is missing though agent-config is fresh" \
  || bad "--bins did not retry a missing bin (cargo calls: $(cat "$T/cargo.log" 2>/dev/null | wc -l))"
cp "$FR/rust/agent-config/target/release/agent-config" "$FR/rust/agent-config/target/release/beta"
rm -f "$T/cargo.log"; bins
[[ ! -e "$T/cargo.log" ]] && pass "--bins builds nothing when every bin is fresh" || bad "--bins built with every bin fresh: $(cat "$T/cargo.log")"
touch -t 202001020000 "$FR/rust/agent-config/src/b.rs"; touch -t 202001030000 "$FR/rust/agent-config/target/release/agent-config"
touch -t 202001010000 "$FR/rust/agent-config/target/release/beta"
rm -f "$T/cargo.log"; bins
[[ "$(grep -c '^cargo build' "$T/cargo.log" 2>/dev/null)" -ge 1 ]] && pass "--bins builds again when one bin is older than its sources" \
  || bad "--bins did not retry a stale bin"

# 13c. with_heartbeat: a silent command that runs long says "still ..." every second (here), keeps its
#      exit code and its output in the log, and names another process that has the lock file open.
#      A fast command prints nothing.
hb() { # hb <lockfile> <log> <cmd...>: stderr of with_heartbeat in $T/hb.err, its rc on stdout
  local lock="$1" log="$2"; shift 2
  DOTS_HEARTBEAT=1 bash -c '. "$1/scripts/setup-lib.sh"; shift; with_heartbeat "doing x" "$@"; echo $?' _ "$R" "$lock" "$log" "$@" 2>"$T/hb.err"
}
rc="$(hb "" "$T/hb.log" sh -c 'sleep 3; echo built; exit 7')"
[[ "$rc" == 7 ]] && pass "with_heartbeat keeps the exit code" || bad "with_heartbeat rc=$rc, wanted 7"
grep -q '^»» still doing x (' "$T/hb.err" && pass "with_heartbeat says it is still working" || bad "with_heartbeat printed no heartbeat: $(cat "$T/hb.err")"
[[ "$(cat "$T/hb.log")" == built && "$(grep -c built "$T/hb.err")" -eq 0 ]] && pass "with_heartbeat keeps the command output in the log" || bad "with_heartbeat output went astray"
rc="$(hb "" "$T/hb.log" true)"
[[ "$rc" == 0 && ! -s "$T/hb.err" ]] && pass "with_heartbeat prints nothing for a fast command" || bad "with_heartbeat fast: rc=$rc err=$(cat "$T/hb.err")"
if command -v lsof >/dev/null 2>&1; then
  : >"$T/hb.lock"; ( exec 9>>"$T/hb.lock"; sleep 6 ) & hbp=$!
  sleep 1; hb "$T/hb.lock" "$T/hb.log" sleep 3 >/dev/null
  grep -q 'Another run (pid [0-9]*) has the build folder open' "$T/hb.err" && pass "with_heartbeat names the process that holds the lock" || bad "with_heartbeat did not name the lock holder: $(cat "$T/hb.err")"
  kill "$hbp" 2>/dev/null; wait "$hbp" 2>/dev/null
fi

# 14. Step 20 (Homebrew apps) on a Mac with Homebrew: a Brewfile whose one formula is installed
#     skips; one with an unknown formula warns and the run goes on. Nothing installs.
if [[ "$(uname -s)" == Darwin ]] && command -v brew >/dev/null 2>&1; then
  f1="$(brew list --formula 2>/dev/null | head -1)"
  cp "$R/Brewfile" "$T/Brewfile.keep"
  if [[ -n "$f1" ]]; then
    printf 'brew "%s"\n' "$f1" >"$R/Brewfile"
    run step20a.log DOTFILES_STEPS=20-brew-bundle
    grep -q 'up to date\|already installed' "$T/step20a.log" && ! grep -q 'Some Brewfile' "$T/step20a.log" \
      && pass "step 20 skips when the Brewfile is satisfied" || bad "step 20 satisfied Brewfile (log: step20a.log): $(tail -n 3 "$T/step20a.log" | tr '\n' '|')"
  fi
  printf 'brew "no-such-formula-dots-test"\n' >"$R/Brewfile"
  run step20b.log DOTFILES_STEPS=20-brew-bundle
  grep -q 'Some Brewfile entries failed' "$T/step20b.log" && grep -q 'fix-brew-permissions.sh' "$T/step20b.log" \
    && grep -q 'done in [0-9hms]* with [0-9]* warnings' "$T/step20b.log" && pass "step 20 warns about a failed Brewfile and goes on" \
    || bad "step 20 failing Brewfile (log: step20b.log): $(tail -n 5 "$T/step20b.log" | tr '\n' '|')"
  ! grep -q '^Using \|brew bundle. complete' "$T/step20b.log" && pass "step 20 hides brew's Using lines" || bad "step 20 printed a Using line"
  cp "$T/Brewfile.keep" "$R/Brewfile"
fi

# 15. Step 50 (Node + pnpm) downloads Node, so it runs only with DOTS_TEST_NETWORK=1 and a mise on PATH.
#     Node, mise and corepack stay in the temp HOME: no real mise shims on PATH, so `corepack enable`
#     cannot write links into the real Node bin folder.
if [[ -n "${DOTS_TEST_NETWORK:-}" ]] && command -v mise >/dev/null 2>&1 && [[ "$(uname -s)" == Darwin ]]; then
  node_path="$(dirname "$(command -v mise)")"
  for d in ${PATH//:/ }; do case "$d" in *mise*|*/.local/share/*) ;; *) node_path="$node_path:$d" ;; esac; done
  real_pnpm="$(mise which pnpm 2>/dev/null || true)"; real_pnpm_to="$(readlink "$real_pnpm" 2>/dev/null || true)"
  iso=(PATH="$node_path" MISE_DATA_DIR="$H/.local/share/mise" MISE_CONFIG_DIR="$H/.config/mise"
       MISE_CACHE_DIR="$H/.cache/mise" COREPACK_HOME="$H/.cache/node/corepack")
  run step50.log DOTFILES_STEPS=50-node-pnpm "${iso[@]}"
  grep -q 'node + pnpm ready\|Node + pnpm (via mise): 2 up to date' "$T/step50.log" && ! grep -q '^!' "$T/step50.log" && pass "step 50 makes Node and pnpm ready" \
    || bad "step 50 (log: step50.log): $(tail -n 5 "$T/step50.log" | tr '\n' '|')"
  run step50b.log DOTFILES_STEPS=50-node-pnpm "${iso[@]}"
  grep -q 'up to date' "$T/step50b.log" && pass "step 50 on a second run changes nothing" || bad "step 50 run 2 (log: step50b.log): $(tail -n 4 "$T/step50b.log" | tr '\n' '|')"
  [[ "$(readlink "$real_pnpm" 2>/dev/null || true)" == "$real_pnpm_to" ]] && pass "step 50 leaves the real pnpm link alone" \
    || bad "step 50 changed the real pnpm link $real_pnpm"
fi

# 15b. The old status line link: a dangling ~/.claude/statusline.sh into the repo goes, a link
#      to somewhere else stays.
ln -s "$R/claude/statusline.sh" "$H/.claude/statusline.sh"
run run15b.log
[[ ! -e "$H/.claude/statusline.sh" && ! -L "$H/.claude/statusline.sh" ]] && pass "the old statusline.sh link into the repo is removed" || bad "old statusline.sh link still there"
grep -q 'removed the old statusline.sh link' "$T/run15b.log" && pass "its removal is reported" || bad "removal not reported"
ln -s "$T/my-statusline.sh" "$H/.claude/statusline.sh"
run run15c.log
[[ "$(readlink "$H/.claude/statusline.sh")" == "$T/my-statusline.sh" ]] && pass "a statusline.sh link elsewhere is left alone" || bad "a foreign statusline.sh link was touched"
rm -f "$H/.claude/statusline.sh"

# 16. No Rust binary: ./dotfiles runs on a small PATH (no python3), and with no Rust binary a
#     hook exits 0 silent while a skill call says "Run: dots" and exits 127.
FP="$T/nopy"; mkdir -p "$FP"
for t in bash env sh cat grep egrep tr id sed uname dirname basename mktemp rm mkdir rmdir chmod date tail head awk ln cp mv ls find xargs \
    tar cksum readlink wc sort cut tee touch git stat realpath expr test which diff cmp curl; do
  p="$(command -v "$t" 2>/dev/null)" && [[ "$p" == /* ]] && ln -sf "$p" "$FP/$t"
done
rm -f "$FP/python3" "$FP/python"
printf 'Test User\ntest@example.com\n' >"$T/stdin"
if run run16.log PATH="$FP"; then pass "./dotfiles runs with no python3 on PATH"; else bad "./dotfiles without python3 (log: run16.log): $(tail -n 3 "$T/run16.log" | tr '\n' '|')"; fi
[[ ! -e "$R/rust/agent-config/target" ]] && pass "the temp repo has no Rust binary" || bad "the temp repo already has a Rust build"
pre='{"tool_name":"Bash","tool_input":{"command":"ls"},"session_id":"t","cwd":"/tmp"}'
out="$(printf '%s' "$pre" | env HOME="$H" PATH="$FP" bash "$R/claude/hooks/rust-bin" --hook guard pretool 2>"$T/hook.err")"; rc=$?
[[ "$rc" -eq 0 && -z "$out" ]] && pass "a hook with no binary exits 0 and prints nothing" || bad "hook with no binary: rc=$rc out='$out'"
out="$(env HOME="$H" PATH="$FP" bash "$R/claude/hooks/rust-bin" route 2>&1 </dev/null)"; rc=$?
[[ "$rc" -eq 127 && "$out" == *"Run: dots"* ]] && pass "a skill call with no binary says Run: dots and exits 127" || bad "skill call with no binary: rc=$rc out='$out'"

# 17. Private words (scripts/git-hooks/private-words.sh), through real commits in a throwaway repo:
#     the current user's home paths, whole-word matching for short words, a warning for an empty list.
PW="$T/pw"; PH="$T/pwhome"
mkdir -p "$PW" "$PH/.config/dotfiles"
tgit -C "$PW" init -q
pw_me="$(id -un 2>/dev/null)"; [[ -n "$pw_me" ]] || pw_me="${USERNAME:-}"
pw_settings() { printf '%s\n' "$@" >"$PH/.config/dotfiles/settings"; }
# Stage a file with the text $2 and commit it with the repo's hooks. Output goes to $T/pw.out.
# A refused commit is unstaged again, so the next case starts clean.
pw_commit() {
  printf '%s\n' "$2" >"$PW/$1"
  tgit -C "$PW" add "$1"
  if HOME="$PH" tgit -C "$PW" -c core.hooksPath="$REPO/scripts/git-hooks" commit -q -m "add $1" >"$T/pw.out" 2>&1; then
    return 0
  fi
  tgit -C "$PW" reset -q
  return 1
}
if [[ -z "$pw_me" ]]; then
  bad "private words: no user name to test the home-path rule (id -un and USERNAME are empty)"
else
  pw_settings 'PRIVATE_WORDS="zebrafish"'
  for p in "/home/$pw_me/x" "/Users/$pw_me/x" "C:\\Users\\$pw_me\\x" "C:/Users/$pw_me/x"; do
    pw_commit homes.txt "see $p" && bad "private words: a staged home path was not blocked: $p" || pass "private words: a staged home path is blocked ($p)"
  done
  pw_commit other.txt "see /home/someone-else/x" && pass "private words: another user's home path passes" || bad "private words: another user's path was blocked: $(cat "$T/pw.out")"
  for p in "file:///home/$pw_me/x" "file:///Users/$pw_me/x" "file://localhost/home/$pw_me/x" "ends at /home/$pw_me"; do
    pw_commit url.txt "see $p" && bad "private words: a home path in a URL or at line end was not blocked: $p" || pass "private words: a home path in a URL or at line end is blocked ($p)"
  done
  # the user name must end there: /home/<user>x is another user's folder
  pw_commit longer.txt "see /home/${pw_me}x/y and /Users/${pw_me}-old/z" && pass "private words: a longer user name passes" || bad "private words: /home/${pw_me}x was blocked: $(cat "$T/pw.out")"
  pw_settings 'PRIVATE_WORDS="abc"'
  pw_commit short1.txt "abc here" && bad "private words: a short word as a whole word was not blocked" || pass "private words: a short word matches as a whole word"
  pw_commit short2.txt "abcdef" && pass "private words: a short word inside a longer word passes" || bad "private words: abc blocked abcdef: $(cat "$T/pw.out")"
  pw_commit short3.txt "x-abc-y" && bad "private words: abc between hyphens was not blocked" || pass "private words: a short word between punctuation is blocked"
  pw_settings 'PRIVATE_WORDS="abcdef"'
  pw_commit long1.txt "xxabcdefxx" && bad "private words: a long word inside text was not blocked" || pass "private words: a word of 5 letters or more matches anywhere"
  # a short word is counted in characters: "ñoño" is 4 characters (6 bytes), in any locale
  pw_settings 'PRIVATE_WORDS="ñoño"'
  for loc in C en_US.UTF-8; do
    LC_ALL="$loc" pw_commit "utf1-$loc.txt" "xñoñox" && pass "private words: a 4-character UTF-8 word inside a longer word passes ($loc)" || bad "private words: ñoño blocked xñoñox ($loc): $(cat "$T/pw.out")"
    LC_ALL="$loc" pw_commit "utf2-$loc.txt" "a ñoño b" && bad "private words: a 4-character UTF-8 word as a whole word was not blocked ($loc)" || pass "private words: a 4-character UTF-8 word matches as a whole word ($loc)"
  done
  # PUBLIC_HOSTS: a listed host passes, the bare word and other paths still block, a hostile entry is ignored
  pw_settings 'PRIVATE_WORDS="zebrafish"' 'PUBLIC_HOSTS="zebrafish.example, a|b"'
  pw_commit host1.txt "see https://zebrafish.example/story and HTTPS://ZEBRAFISH.EXAMPLE" && pass "private words: a listed public host passes" || bad "private words: a listed host was blocked: $(cat "$T/pw.out")"
  pw_commit host2.txt "I like zebrafish" && bad "private words: the bare word was not blocked with a host listed" || pass "private words: the bare word still blocks with a host listed"
  pw_commit host3.txt "see https://zebrafish.example/zebrafish" && bad "private words: a word in the path of a listed host was not blocked" || pass "private words: a word in a path still blocks"
  pw_commit host4.txt "see https://other.example/a|b zebrafish.net" && bad "private words: another host with the word was not blocked" || pass "private words: another host holding the word still blocks"
  pw_settings 'PRIVATE_WORDS="zebrafish"'
  pw_commit host5.txt "see https://zebrafish.example/story" && bad "private words: the host passed with PUBLIC_HOSTS unset" || pass "private words: the host is blocked with PUBLIC_HOSTS unset"
  pw_settings ''
  pw_commit empty.txt "plain text" && pass "private words: with no words a commit passes" || bad "private words: a commit with no words failed: $(cat "$T/pw.out")"
  [[ "$(grep -c 'private words: none set' "$T/pw.out")" -ge 1 ]] && pass "private words: an empty list prints a warning" || bad "private words: no warning for an empty list: $(cat "$T/pw.out")"
fi

if [[ "$fails" -eq 0 ]]; then printf 'ALL PASS (%d checks)\n' "$passes"; exit 0; fi
KEEP=1
printf '%d of %d check(s) failed\n' "$fails" "$((fails + passes))"
exit 1
