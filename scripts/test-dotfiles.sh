#!/usr/bin/env bash
# ============================================================================
#  scripts/test-dotfiles.sh — run ./dotfiles in a temp HOME and check what it
#  links and saves. Mac and Linux, no container, about 10 seconds.
#
#  It copies the tracked files of this working tree (your edits included) into
#  a temp repo with a local "origin", then runs ./dotfiles with no install.d
#  steps (DOTFILES_STEPS=none) and no terminal, so nothing installs or asks.
#  Checks: first run, the owner profile saved, the owner rules linked and
#  unlinked, a run that changes nothing, and the restart after a pull.
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

T="$(mktemp -d)"
T="$(cd "$T" && pwd -P)"
cleanup() { if [[ -n "$KEEP" ]]; then printf 'kept: %s\n' "$T"; else rm -rf "$T"; fi; }
trap cleanup EXIT

fails=0 passes=0
pass() { passes=$((passes + 1)); if [[ -n "$VERBOSE" ]]; then printf 'PASS  %s\n' "$*"; fi; }
bad()  { printf 'FAIL  %s\n' "$*"; fails=$((fails + 1)); }

# git for the setup only: no user config, no hooks, no signing.
tgit() {
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git -c user.name=Test -c user.email=test@example.com \
    -c core.hooksPath=/dev/null -c commit.gpgsign=false -c init.defaultBranch=main "$@"
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
  env HOME="$H" NO_COLOR=1 DOTFILES_STEPS=none "$@" bash "$R/dotfiles" >"$T/$log" 2>&1 <"$T/stdin"
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
python3 -c 'import json,sys; sys.exit(json.load(open(sys.argv[1])) != json.load(open(sys.argv[2])))' \
  "$LIVE" "$R/claude/settings.json" && pass "settings.json equals the repo base" || bad "settings.json differs from the base"
[[ ! -e "$LAYER" ]] && pass "no private layer yet" || bad "a private layer appeared on its own"
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
python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); d["testKeptKey"]=True; open(p,"w").write(json.dumps(d, indent=4))' "$LIVE"
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

# 6. A new commit on origin: the run pulls it and restarts with the new script.
tgit clone -q "$T/origin.git" "$T/other" &&
  sed 's/ok "dotfiles: done/ok "dotfiles: done (new)/' "$T/other/dotfiles" >"$T/dotfiles.new" &&
  cat "$T/dotfiles.new" >"$T/other/dotfiles" &&
  tgit -C "$T/other" commit -qam "new script" && tgit -C "$T/other" push -q origin HEAD:main ||
  bad "couldn't push a new commit to the temp origin"
run run5.log
grep -q 'pulled the latest version' "$T/run5.log" && pass "pull restarts the script" || bad "no restart after the pull"
grep -q 'dotfiles: done (new)' "$T/run5.log" && pass "the new script ran" || bad "the old script ran after the pull"

# 7. Windows safety: no tracked path may hold a name Windows cannot check out.
. "$REPO/scripts/git-hooks/windows-paths.sh"
winbad="$(cd "$REPO" && git ls-files | windows_bad_paths)"
[[ -z "$winbad" ]] && pass "every tracked path is Windows-safe" || { bad "paths Windows cannot hold: $(printf '%s' "$winbad" | head -3 | tr '\n' ' ')"; }

# 8. bootstrap.sh: a fresh clone, and the repair of a clone with no checkout (what
#    a failed Windows clone leaves behind).
H2="$T/home2"; mkdir -p "$H2"
printf 'Test User\ntest@example.com\n' >"$T/stdin"
bs() { # bs <log> <dir>
  env HOME="$H2" NO_COLOR=1 DOTFILES_STEPS=none GIT_CONFIG_GLOBAL=/dev/null DOTFILES_REPO="$T/origin.git" DOTFILES="$2" \
    bash "$REPO/bootstrap.sh" >"$T/$1" 2>&1 <"$T/stdin"
}
if bs bs1.log "$T/bs1" && [[ -f "$T/bs1/dotfiles" ]]; then pass "bootstrap: fresh clone runs"; else bad "bootstrap: fresh clone (log: bs1.log)"; fi
[[ "$(git -C "$T/bs1" config core.autocrlf)" == false ]] && pass "bootstrap: autocrlf off in the clone" || bad "bootstrap: autocrlf not off"
tgit clone -q --no-checkout "$T/origin.git" "$T/bs2" && [[ ! -f "$T/bs2/dotfiles" ]] || bad "couldn't make a clone without a checkout"
if bs bs2.log "$T/bs2" && [[ -f "$T/bs2/dotfiles" ]]; then pass "bootstrap: a clone with no checkout is repaired"; else bad "bootstrap: repair (log: bs2.log)"; fi
grep -q 'Repairing' "$T/bs2.log" && pass "bootstrap: says it repairs" || bad "bootstrap: repair not reported"
rc=0; DOTFILES_REPO="$T/missing.git" DOTFILES="$T/bs3" HOME="$H2" GIT_CONFIG_GLOBAL=/dev/null bash "$REPO/bootstrap.sh" >"$T/bs3.log" 2>&1 <"$T/stdin" || rc=$?
[[ "$rc" -ne 0 && ! -e "$T/bs3" ]] && pass "bootstrap: a failed clone leaves nothing behind" || bad "bootstrap: failed clone left $T/bs3 or exited 0"

# 9. A Windows run (Git Bash), faked with a `uname` that says MINGW64: it runs only the
#    Windows steps, links little, prints one total, and leaves the user's own .gitconfig alone.
H3="$T/home3"; mkdir -p "$H3" "$T/fakebin"
printf '#!/bin/sh\ncase "$1" in -s) echo MINGW64_NT-10.0 ;; *) exec /usr/bin/uname "$@" ;; esac\n' >"$T/fakebin/uname"
chmod +x "$T/fakebin/uname"
printf '[user]\n\tname = Mine\n' >"$H3/.gitconfig.bak-1"           # the user's own file, backed up by an old run
cp "$R/home/.gitconfig" "$H3/.gitconfig"                              # the repo copy an old run left
printf 'Win User\nwin@example.com\n' >"$T/stdin"
rc=0
env HOME="$H3" PATH="$T/fakebin:$PATH" NO_COLOR=1 AGENT_CONFIG_IMPL=python bash "$R/dotfiles" >"$T/win.log" 2>&1 <"$T/stdin" || rc=$?
[[ "$rc" -eq 0 ]] && pass "Windows run exits 0" || bad "Windows run exit $rc (log: win.log)"
grep -qiE 'homebrew|oh my zsh|brew bundle' "$T/win.log" && bad "Windows run mentions Homebrew or Oh My Zsh" || pass "Windows run is quiet about Mac tools"
grep -q 'All set' "$T/win.log" && pass "Windows run ends with the All set box" || bad "Windows run has no All set box"
[[ "$(grep -c '^✓ linked ' "$T/win.log")" -le 4 ]] && pass "Windows run prints totals, not one line per link" || bad "Windows run prints many link lines"
[[ -L "$H3/.claude/CLAUDE.md" ]] && pass "Windows run links ~/.claude" || bad "Windows run did not link ~/.claude"
[[ ! -L "$H3/.gitconfig" ]] && grep -q 'name = Mine' "$H3/.gitconfig" && pass "Windows run keeps the user's own .gitconfig" || bad "Windows run replaced the user's .gitconfig"
git config -f "$H3/.gitconfig" --get-all include.path | grep -q gitconfig.local && pass "Windows .gitconfig includes the identity file" || bad "Windows .gitconfig misses the include"
[[ ! -e "$H3/.zshrc" ]] && pass "Windows run does not link .zshrc" || bad "Windows run linked .zshrc"

# 10. The last line: the done line, or the warning count of every step. A test step in the
#     temp repo warns; the count file is gone after the run.
mkdir -p "$T/tmp"
[[ "$(tail -n 1 "$T/run3.log")" =~ ^✓\ dotfiles:\ done\ \(profile:\ [a-z]+\)$ ]] && pass "a clean run ends with the done line" \
  || bad "clean run last line: $(tail -n 1 "$T/run3.log")"
printf '#!/usr/bin/env bash\n. "$REPO/scripts/setup-lib.sh"\nwarn "test warning one"\nwarn "test warning two"\n' \
  >"$R/scripts/install.d/99-test-warn.sh"
run run10.log DOTFILES_STEPS=99-test-warn TMPDIR="$T/tmp"
rm -f "$R/scripts/install.d/99-test-warn.sh"
grep -q '^!  dotfiles: done with 2 warnings (profile: ' "$T/run10.log" && pass "the last line counts the warnings of a step" \
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
    apt-get) want_list="build-essential ca-certificates curl file git procps python3 zsh"
      want_log="apt-get install -y -qq --no-install-recommends zsh" ;;
    dnf) want_list="gcc gcc-c++ make ca-certificates curl file git procps-ng python3 zsh"; want_log="dnf install -y -q zsh" ;;
    pacman) want_list="base-devel ca-certificates curl file git procps-ng python zsh"
      want_log="pacman -S --needed --noconfirm zsh" ;;
    apk) want_list="bash ca-certificates curl file git python3 zsh"; want_log="apk add -q --no-cache zsh" ;;
  esac
  [[ "$out" == "$m|$want_list|zsh |rc=0" ]] && pass "Linux $m: finds the manager, its list and the missing zsh" \
    || bad "Linux $m: got '$out'"
  grep -qxF "$want_log" "$d/log" 2>/dev/null && pass "Linux $m: installs zsh with the right command" \
    || bad "Linux $m: install command: $(cat "$d/log" 2>/dev/null | tr '\n' ';')"
done
d="$T/linux-none"; fake_linux "$d" none; out="$(lx "$d")"
[[ "$out" == "|cc make curl file git ps python3 zsh|cc make curl file git ps python3 zsh |rc=1" ]] \
  && pass "Linux with no known manager: lists the missing commands and fails" || bad "Linux with no manager: got '$out'"

# 12. The os-check skip rule: which OS each changed file is for.
which_want='dotfiles: macos linux windows
scripts/setup-lib.sh: macos linux windows
rust/agent-config/src/lib.rs: macos linux windows
claude/hooks/guard.py: macos linux windows
scripts/install.d/15-winget.sh: windows
scripts/install.d/57-claude-code.sh: macos linux windows
scripts/install.d/70-macos-defaults.sh: macos
scripts/install.d/05-linux-prereqs.sh: linux
scripts/install.d/78-linux-extras.sh: linux
Brewfile: macos linux
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

if [[ "$fails" -eq 0 ]]; then printf 'ALL PASS (%d checks)\n' "$passes"; exit 0; fi
KEEP=1
printf '%d of %d check(s) failed\n' "$fails" "$((fails + passes))"
exit 1
