#!/usr/bin/env bash
# private.sh: the private settings. They stay in ~/.config/dotfiles (mode 600), never in git:
#   owner.md       private rules for Claude and Cursor (dots links it as
#                  ~/.claude/rules/owner.md and adds it to the Cursor rule)
#   private-words  words the repo's git hooks block in commits (scripts/git-hooks/private-words.sh)
# ./dotfiles runs this on the first run at a terminal. Any time later: dots private
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

WORDS="$DOTFILES_CONF/private-words"
keep="⏭️  Keep them" change="✏️  Change them" skip="⏭️  No, skip it" add="✏️  Add them now"

# save <file> <what> <text>: store the text (mode 600). Empty text removes the file.
save() {
  if [[ -z "${3//[[:space:]]/}" ]]; then
    rm -f "$1" && info "$2: none on this Mac"
  else
    (umask 077 && printf '%s\n' "$3" >"$1") && chmod 600 "$1" &&
      ok "$2 saved in ~/.config/dotfiles/$(basename "$1")"
  fi
}

# edit_file <file> <what> <header> [text]: the text box, then save. Esc keeps the file.
edit_file() {
  local text
  if text="$(ask_long "$3" "${4:-}")"; then save "$1" "$2" "$text"; else info "$2: not changed"; fi
}

step "Private settings (they stay on this Mac, never in git)"
mkdir -p "$DOTFILES_CONF" && chmod 700 "$DOTFILES_CONF"

# 1. Private rules: Claude and Cursor read them in every session on this Mac.
if [[ -s "$OWNER_MD" ]]; then
  lines="$(wc -l <"$OWNER_MD" | tr -d ' ')"
  if [[ "$(ask_one "🔒 Your private rules for Claude and Cursor: $lines lines" "$keep" "$change")" == "$change" ]]; then
    edit_file "$OWNER_MD" "private rules" "🔒 Private rules (Markdown)" "$(cat "$OWNER_MD")"
  fi
else
  info "Private rules are extra rules for Claude and Cursor, only on this Mac."
  info "Have them on another Mac? Run there: pbcopy < ~/.config/dotfiles/owner.md"
  if [[ "$(ask_one "🔒 Add private rules for Claude and Cursor?" "$skip" "$add")" == "$add" ]]; then
    edit_file "$OWNER_MD" "private rules" "🔒 Private rules (Markdown). Paste or type them."
  fi
fi

# 2. Private words: the repo's git hooks block a commit that holds one.
if [[ -s "$WORDS" ]]; then
  n="$(grep -cvE '^[[:space:]]*(#|$)' "$WORDS")"
  if [[ "$(ask_one "🚫 Your private words: $n" "$keep" "$change")" == "$change" ]]; then
    edit_file "$WORDS" "private words" "🚫 Private words, one per line" "$(cat "$WORDS")"
  fi
else
  info "Private words never go into a commit of this repo. The git hooks block them."
  if [[ "$(ask_one "🚫 Add private words? Your git name and email are filled in." "$add" "$skip")" == "$add" ]]; then
    value="# Private words, one per line. Lines that start with # are notes."
    for v in "$(git config -f "$HOME/.gitconfig.local" user.name 2>/dev/null)" \
             "$(git config -f "$HOME/.gitconfig.local" user.email 2>/dev/null)"; do
      if [[ -n "$v" ]]; then value+=$'\n'"$v"; fi
    done
    edit_file "$WORDS" "private words" "🚫 Private words, one per line" "$value"
  fi
fi
