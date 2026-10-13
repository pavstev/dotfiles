# shellcheck shell=bash
# Sourced by the pre-commit and commit-msg hooks. Not a hook itself.
# The repo is impersonal (AGENTS.md "Conventions"). The private words come from the
# settings (dots settings): PRIVATE_PROJECTS and PRIVATE_WORDS, plus the git email,
# the full git name and that name written as one word.
. "$(dirname "${BASH_SOURCE[0]}")/../setup-lib.sh"

# private_words: the words, one per line. A word made from the git name or email
# that is part of the origin URL (the GitHub handle in the clone URL) is left out.
private_words() {
  local name email url w
  local -a list=()
  read -r -a list <<<"$(setting PRIVATE_PROJECTS) $(setting PRIVATE_WORDS)"
  name="$(setting GIT_NAME)" email="$(setting GIT_EMAIL)"
  url="$(git remote get-url origin 2>/dev/null | tr '[:upper:]' '[:lower:]')"
  {
    for w in ${list[@]+"${list[@]}"}; do printf '%s\n' "$w"; done
    for w in "$email" "$name" "${name// /}"; do
      w="$(printf '%s' "$w" | tr '[:upper:]' '[:lower:]')"
      if [[ ${#w} -ge 3 && ( -z "$url" || "$url" != *"$w"* ) ]]; then printf '%s\n' "$w"; fi
    done
  } | awk 'NF && !seen[tolower($0)]++'
}

# private_hits: print the stdin lines that hold a private word (any case).
private_hits() {
  local pats
  pats="$(private_words)"
  [[ -n "$pats" ]] || return 0
  grep -iF -- "$pats" || true
}
