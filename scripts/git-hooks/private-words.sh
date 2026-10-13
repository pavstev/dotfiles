# Sourced by the pre-commit and commit-msg hooks. Not a hook itself.
# The repo is impersonal (AGENTS.md "Conventions"). The owner lists private words
# (names, emails, accounts, private project names) outside the repo, one per line.
PRIVATE_WORDS="$HOME/.config/dotfiles/private-words"

# private_hits: print the stdin lines that hold a listed word (any case).
# The list may have # comments, blank lines, CRLF ends and extra spaces.
private_hits() {
  local pats
  [[ -s "$PRIVATE_WORDS" ]] || return 0
  pats="$(tr -d '\r' <"$PRIVATE_WORDS" \
    | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' -e '/^#/d' -e '/^$/d')"
  [[ -n "$pats" ]] || return 0
  grep -iF -- "$pats" || true
}
