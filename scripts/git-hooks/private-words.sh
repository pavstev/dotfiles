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

# private_user: the login name (Windows Git Bash: $USERNAME when id fails).
private_user() {
  local u
  u="$(id -un 2>/dev/null)" || u=""
  [[ -n "$u" ]] || u="${USERNAME:-}"
  printf '%s' "$u"
}

# word_chars: the length of $1 in characters, whatever the locale (a UTF-8 byte that continues
# a character is not counted), so "ñoño" is 4 and a hook run under LANG=C agrees with one under UTF-8.
word_chars() {
  local n
  n="$(printf '%s' "$1" | LC_ALL=C tr -d '\200-\277' | LC_ALL=C wc -c)"
  printf '%d' "$((n))"
}

# home_path_hits: print the stdin lines that hold a home path of the user $1 (/Users/<user>,
# /home/<user>, C:\Users\<user>, C:/Users/<user>; also inside file:// URLs). The path must end
# there: /Users/dana does not match /Users/danaq (a letter, digit, _ or - goes on).
home_path_hits() {
  PW_USER="$1" awk '
    function hit(line, pat,   l, s, i, pos, c) {
      l = tolower(line); pat = tolower(pat); s = 1
      while ((i = index(substr(l, s), pat)) > 0) {
        pos = s + i - 1
        c = substr(l, pos + length(pat), 1)
        if (c == "" || c !~ /[A-Za-z0-9_-]/) return 1
        s = pos + 1
      }
      return 0
    }
    BEGIN { u = ENVIRON["PW_USER"] }
    hit($0, "/Users/" u) || hit($0, "/home/" u) || hit($0, "C:\\Users\\" u) || hit($0, "C:/Users/" u)
  '
}

# private_hits: print the stdin lines that hold a private word (any case) or a home path of the
# current user. A word of 4 characters or fewer matches whole words only (grep -w), a longer one
# anywhere. An empty word list prints one warning on stderr (it never blocks); the home-path rule
# still runs.
private_hits() {
  local input words user w
  input="$(cat)"
  words="$(private_words)"
  user="$(private_user)"
  if [[ -z "$words" ]]; then
    printf 'warning: private words: none set (dots settings: PRIVATE_WORDS, PRIVATE_PROJECTS, or a git name), so no word check ran\n' >&2
  fi
  {
    if [[ -n "$user" ]]; then
      printf '%s\n' "$input" | home_path_hits "$user" || true
    fi
    while IFS= read -r w; do
      [[ -n "$w" ]] || continue
      if (( $(word_chars "$w") <= 4 )); then
        printf '%s\n' "$input" | grep -iwF -- "$w" || true
      else
        printf '%s\n' "$input" | grep -iF -- "$w" || true
      fi
    done <<<"$words"
  } | awk '!seen[$0]++'
}
