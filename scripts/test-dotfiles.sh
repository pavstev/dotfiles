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
# ============================================================================
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEEP=""
for a in "$@"; do
  case "$a" in
    --keep) KEEP=1 ;;
    *) printf 'usage: test-dotfiles.sh [--keep]\n' >&2; exit 2 ;;
  esac
done

T="$(mktemp -d)"
T="$(cd "$T" && pwd -P)"
cleanup() { if [[ -n "$KEEP" ]]; then printf 'kept: %s\n' "$T"; else rm -rf "$T"; fi; }
trap cleanup EXIT

fails=0
pass() { printf 'PASS  %s\n' "$*"; }
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

# run <log> [env...]: ./dotfiles in the temp HOME, no terminal, no steps.
run() {
  local log="$1"; shift
  env HOME="$H" NO_COLOR=1 DOTFILES_STEPS=none "$@" bash "$R/dotfiles" >"$T/$log" 2>&1 <"$T/stdin"
}
# fingerprint: every link under the temp HOME and the saved settings.
fingerprint() {
  { find "$H" -type l 2>/dev/null | sort | while IFS= read -r l; do printf '%s -> %s\n' "$l" "$(readlink "$l")"; done
    cat "$SETTINGS" "$H/.gitconfig.local" "$LIVE" "$LAYER" 2>/dev/null
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

# 3. Private rules appear: the owner profile is saved and the rules link.
: >"$T/stdin"
printf '# Owner rules\n' >"$OWNER"
if run run2.log; then pass "owner run exits 0"; else bad "owner run exit $?"; fi
grep -q '^PROFILE="owner"' "$SETTINGS" && pass "owner profile saved" || bad "owner profile not saved"
[[ "$(readlink "$RULE")" == "$OWNER" ]] && pass "owner rules linked" || bad "owner rules not linked"

# 4. Same again: nothing changes.
before="$(fingerprint)"
run run3.log
after="$(fingerprint)"
[[ "$before" == "$after" ]] && pass "second run changes nothing" || bad "second run changed links or settings"
grep -qE 'linked |backed up' "$T/run3.log" && bad "second run linked or backed up something" || pass "second run links nothing new"

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
  sed 's/ok "dotfiles: done"/ok "dotfiles: done (new)"/' "$T/other/dotfiles" >"$T/dotfiles.new" &&
  cat "$T/dotfiles.new" >"$T/other/dotfiles" &&
  tgit -C "$T/other" commit -qam "new script" && tgit -C "$T/other" push -q origin HEAD:main ||
  bad "couldn't push a new commit to the temp origin"
run run5.log
grep -q 'pulled the latest version' "$T/run5.log" && pass "pull restarts the script" || bad "no restart after the pull"
grep -q 'dotfiles: done (new)' "$T/run5.log" && pass "the new script ran" || bad "the old script ran after the pull"

if [[ "$fails" -eq 0 ]]; then printf '\nALL PASS\n'; exit 0; fi
KEEP=1
printf '\n%d check(s) failed\n' "$fails"
exit 1
