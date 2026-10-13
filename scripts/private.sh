#!/usr/bin/env bash
# private.sh: the private settings. They never go into git:
#   ~/.config/dotfiles/owner.md       private rules for Claude and Cursor (dots links it as
#                                     ~/.claude/rules/owner.md and adds it to the Cursor rule)
#   ~/.config/dotfiles/private-words  words the repo's git hooks block in commits
#   home/.config/zsh/local.zsh        (ignored by git) JAVA_HOME, in a marked block
# Files are mode 600. ./dotfiles runs this on the first run at a terminal.
# Any time later: dots private
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

WORDS="$DOTFILES_CONF/private-words"
LOCAL_ZSH="$REPO/home/.config/zsh/local.zsh"   # ~/.config/zsh links to this folder
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

# --- Java helpers -------------------------------------------------------------
JAVA_BEGIN="# >>> dots private: java >>>"
JAVA_END="# <<< dots private: java <<<"

# java_home_of <dir>: the Java home at or in <dir> (the folder with bin/java), or
# nothing. Homebrew keeps it in libexec/openjdk.jdk/Contents/Home, a .jdk folder in
# Contents/Home. /usr is skipped: its java is the macOS stub.
java_home_of() {
  local d
  for d in "$1/libexec/openjdk.jdk/Contents/Home" "$1/Contents/Home" "$1"; do
    if [[ "$d" != /usr && -f "$d/bin/java" && -x "$d/bin/java" ]]; then
      printf '%s\n' "$d"
      return 0
    fi
  done
}

# java_real <home>: the real folder behind a home, links followed. Two paths with
# the same real folder are the same Java.
java_real() {
  local f="$1/bin/java" l n=0
  while [[ -L "$f" ]] && (( n++ < 20 )); do
    l="$(readlink "$f")"
    if [[ "$l" == /* ]]; then f="$l"; else f="$(dirname "$f")/$l"; fi
  done
  (cd -P "$(dirname "$f")/.." 2>/dev/null && pwd -P)
}

# java_homes: the Java homes in the usual places, one per line, each Java once.
# The first path found wins, so a link that survives updates (Homebrew's
# opt/openjdk@21, mise's java/21) beats the versioned folder behind it.
java_homes() {
  local d h
  {
    /usr/libexec/java_home -V 2>&1 | sed -n 's/^[[:space:]].*[[:space:]]\(\/.*\)$/\1/p'
    for d in /Library/Java/JavaVirtualMachines/* "$HOME"/Library/Java/JavaVirtualMachines/* \
      "${HOMEBREW_PREFIX:-/opt/homebrew}"/opt/openjdk* /usr/local/opt/openjdk* \
      "${MISE_DATA_DIR:-$HOME/.local/share/mise}"/installs/java/* \
      "${SDKMAN_DIR:-$HOME/.sdkman}"/candidates/java/* \
      "${ASDF_DATA_DIR:-$HOME/.asdf}"/installs/java/* "${JENV_ROOT:-$HOME/.jenv}"/versions/* \
      "$HOME"/.gradle/jdks/* "$HOME"/.gradle/jdks/*/* "$HOME"/.jbang/cache/jdks/* \
      "/Applications/Android Studio.app/Contents/jbr" "${JAVA_HOME:-}"; do
      printf '%s\n' "$d"
    done
  } | while IFS= read -r d; do
    if [[ -n "$d" ]]; then h="$(java_home_of "$d")"; else h=""; fi
    if [[ -n "$h" ]]; then printf '%s\t%s\n' "$(java_real "$h")" "$h"; fi
  done | awk -F '\t' '!seen[$1]++ { print $2 }'
}

# java_info <home>: "<major> <tab> <version> <tab> <vendor>", from its release file.
java_info() {
  local v vendor major
  v="$(sed -n 's/^JAVA_VERSION="\(.*\)"/\1/p' "$1/release" 2>/dev/null)"
  vendor="$(sed -n 's/^IMPLEMENTOR="\(.*\)"/\1/p' "$1/release" 2>/dev/null)"
  [[ -n "$v" ]] || v="$("$1/bin/java" -version 2>&1 | sed -n '1s/.*version "\([^"]*\)".*/\1/p')"
  major="${v%%[._+-]*}"
  if [[ "$major" == 1 ]]; then major="${v#1.}"; major="${major%%[._+-]*}"; fi   # 1.8.0 is 8
  [[ "$major" =~ ^[0-9]+$ ]] || major=0
  printf '%s\t%s\t%s\n' "$major" "${v:-unknown}" "$vendor"
}

# java_where <home>: where a Java home came from, in a few words.
# shellcheck disable=SC2088  # the ~ in a label is meant to show as is
java_where() {
  case "$1" in
    */Cellar/openjdk*|*/opt/openjdk*) echo "Homebrew" ;;
    "$HOME"/Library/Java/*) echo "~/Library/Java" ;;
    /Library/Java/*) echo "/Library/Java" ;;
    */mise/installs/*) echo "mise" ;;
    */.sdkman/*|*/candidates/java/*) echo "SDKMAN" ;;
    */.asdf/*) echo "asdf" ;;
    */.jenv/*) echo "jenv" ;;
    */.gradle/jdks/*) echo "Gradle" ;;
    */.jbang/*) echo "jbang" ;;
    *"/Android Studio"*) echo "Android Studio" ;;
    *) echo "$1" ;;
  esac
}

# java_label <home>: "Java 21.0.5, Eclipse Adoptium, /Library/Java".
java_label() {
  local major v vendor where
  IFS=$'\t' read -r major v vendor <<<"$(java_info "$1")"
  where="$(java_where "$1")"
  if [[ "$vendor" == "$where" ]]; then vendor=""; fi
  printf 'Java %s%s, %s' "$v" "${vendor:+, $vendor}" "$where"
}

# java_ranked: the Java homes, best first: long-term-support versions (8, 11, 17,
# 21, 25, ...) before the others, and the newest first in each group.
java_ranked() {
  local h major v vendor lts b c
  java_homes | while IFS= read -r h; do
    IFS=$'\t' read -r major v vendor <<<"$(java_info "$h")"
    IFS='._+-' read -r _ b c _ <<<"${v#1.}"   # 21.0.5: 0 and 5; 1.8.0_432: 0 and 432
    [[ "$b" =~ ^[0-9]+$ ]] || b=0
    [[ "$c" =~ ^[0-9]+$ ]] || c=0
    lts=0
    if (( major == 8 || major == 11 || (major >= 17 && (major - 17) % 4 == 0) )); then lts=1; fi
    printf '%s\t%s\t%s\n' "$lts" "$(( major * 1000000 + 10#$b * 1000 + 10#$c ))" "$h"
  done | sort -s -t $'\t' -k1,1nr -k2,2nr | cut -f3
}

# java_now: JAVA_HOME from the marked block in local.zsh, if any.
java_now() {
  [[ -f "$LOCAL_ZSH" ]] || return 0
  awk -v b="$JAVA_BEGIN" -v e="$JAVA_END" '
    $0 == b { f = 1; next }
    $0 == e { f = 0 }
    f && sub(/^export JAVA_HOME="/, "") { sub(/"$/, ""); v = $0 }
    END { if (v != "") print v }' "$LOCAL_ZSH"
}

# set_java <home>: write the marked JAVA_HOME block at the end of local.zsh (mode
# 600). An empty home removes the block. Lines outside the block stay as they are.
set_java() {
  local tmp
  if [[ ! -f "$LOCAL_ZSH" && -z "$1" ]]; then return 0; fi
  mkdir -p "$(dirname "$LOCAL_ZSH")" && tmp="$(mktemp)" || return 1
  if [[ -f "$LOCAL_ZSH" ]] && ! awk -v b="$JAVA_BEGIN" -v e="$JAVA_END" \
    '$0 == b { s = 1 } !s { print } $0 == e { s = 0 }' "$LOCAL_ZSH" >"$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  if [[ -n "$1" ]]; then
    printf '%s\nexport JAVA_HOME="%s"\n%s\n' "$JAVA_BEGIN" "$1" "$JAVA_END" >>"$tmp"
  fi
  mv "$tmp" "$LOCAL_ZSH" && chmod 600 "$LOCAL_ZSH"
}

# use_java <home>: save it as the Java for new shells, and say so.
use_java() {
  case "$1" in
    *[\"\$\`\\]*) warn "the path holds \" \$ \` or \\, so the Java setting is not changed: $1"
      return 0 ;;
  esac
  set_java "$1" && ok "Java set: $(java_label "$1"). New terminal windows use it."
}

# clean_path <text>: a typed path made usable: no quotes or spaces around it, "\ "
# as a space, ~ as the home folder, no / or /bin/java or /bin at the end.
clean_path() {
  local p="$1"
  p="${p#"${p%%[![:space:]]*}"}"; p="${p%"${p##*[![:space:]]}"}"
  if [[ "$p" == \"*\" || "$p" == \'*\' ]]; then p="${p:1:${#p}-2}"; fi
  p="${p//\\ / }"
  # shellcheck disable=SC2088  # match a typed ~, do not expand it
  if [[ "$p" == "~" || "$p" == "~/"* ]]; then p="$HOME${p:1}"; fi
  while [[ "$p" == */ && "$p" != / ]]; do p="${p%/}"; done
  p="${p%/bin/java}"; p="${p%/bin}"
  printf '%s' "$p"
}

step "Private settings (they stay on this Mac, never in git)"
mkdir -p "$DOTFILES_CONF" && chmod 700 "$DOTFILES_CONF"

# 1. Private rules: Claude and Cursor read them in every session on this Mac.
if [[ -s "$OWNER_MD" ]]; then
  lines="$(wc -l <"$OWNER_MD" | tr -d ' ')"
  if [[ "$(ask_one "🔒 Your private rules for Claude and Cursor: $lines lines" "$keep" "$change")" == "$change" ]]; then
    edit_file "$OWNER_MD" "private rules" "🔒 Private rules (Markdown)" "$(cat "$OWNER_MD")"
  else
    info "private rules: kept ($lines lines)"
  fi
else
  info "Private rules are extra rules for Claude and Cursor, only on this Mac."
  if [[ "$(ask_one "🔒 Add private rules for Claude and Cursor?" "$skip" "$add")" == "$add" ]]; then
    edit_file "$OWNER_MD" "private rules" "🔒 Private rules (Markdown). Paste or type them."
  else
    info "private rules: none. Add them later with: dots private"
  fi
fi

# 2. Private words: the repo's git hooks block a commit that holds one.
if [[ -s "$WORDS" ]]; then
  n="$(grep -cvE '^[[:space:]]*(#|$)' "$WORDS")"
  if [[ "$(ask_one "🚫 Your private words: $n" "$keep" "$change")" == "$change" ]]; then
    edit_file "$WORDS" "private words" "🚫 Private words, one per line" "$(cat "$WORDS")"
  else
    info "private words: kept ($n)"
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
  else
    info "private words: none. Add them later with: dots private"
  fi
fi

# 3. Java: JAVA_HOME for new shells, in a marked block at the end of local.zsh.
# The likely answer comes first: the setting from here, else the Java in use now,
# else the newest long-term-support Java.
labels=() homes=() seen=$'\n'
add_java() {  # <label> <home>: one menu line per Java
  local r
  r="$(java_real "$2")"
  if [[ "$seen" == *$'\n'"$r"$'\n'* ]]; then return 0; fi
  seen+="$r"$'\n'
  labels+=("$1")
  homes+=("$2")
}
now="$(java_now)" gone="" used=""
if [[ -n "$now" && -z "$(java_home_of "$now")" ]]; then
  info "the Java setting points to a folder that is gone: $now"
  gone="$now" now=""
fi
if [[ -z "$now" ]]; then
  info "JAVA_HOME tells Java tools (Maven, Gradle, editors) which Java to use."
  if [[ -n "${JAVA_HOME:-}" ]]; then used="$(java_home_of "$JAVA_HOME")"; fi
fi
if [[ -n "$now" ]]; then add_java "⏭️  Keep it: $(java_label "$now")" "$now"; fi
if [[ -n "$used" ]]; then add_java "☕ $(java_label "$used") (in use now)" "$used"; fi
while IFS= read -r h; do add_java "☕ $(java_label "$h")" "$h"; done < <(java_ranked)
typed="✏️  Type a path"
if [[ -n "$now$gone" ]]; then none="🗑️  Remove the Java setting"; else none="⏭️  No, skip it"; fi
if (( ${#homes[@]} == 0 )); then
  info "No Java found in the usual places."
  labels=("$none" "$typed")
else
  labels+=("$typed" "$none")
fi
if [[ -f "$LOCAL_ZSH" ]] && awk -v b="$JAVA_BEGIN" -v e="$JAVA_END" \
  '$0 == b { s = 1 } !s && /^[^#]*JAVA_HOME=/ { f = 1 } $0 == e { s = 0 } END { exit !f }' "$LOCAL_ZSH"; then
  info "Your own lines in local.zsh also set JAVA_HOME. The block from here goes after them, so it wins."
fi

pick="$(ask_one "☕ Which Java should this Mac use? (sets JAVA_HOME)" "${labels[@]}")"
java=""
for i in "${!labels[@]}"; do
  if (( i < ${#homes[@]} )) && [[ "${labels[$i]}" == "$pick" ]]; then java="${homes[$i]}"; break; fi
done
if [[ -n "$java" && "$java" == "$now" ]]; then
  info "Java: kept ($(java_label "$now"))"
elif [[ -n "$java" ]]; then
  use_java "$java"
elif [[ "$pick" == "$typed" ]]; then
  p="$(clean_path "$(ask_text "☕ Path to a Java home (the folder with bin/java). Empty: no change." \
    "/Library/Java/JavaVirtualMachines/<name>.jdk/Contents/Home" empty-ok)")"
  h=""
  if [[ -n "$p" ]]; then h="$(java_home_of "$p")"; fi
  if [[ -z "$p" ]]; then
    info "Java: not changed"
  elif [[ -z "$h" ]]; then
    warn "no bin/java in $p, so the Java setting is not changed"
  else
    use_java "$h"
  fi
elif [[ "$pick" == "$none" && -n "$now$gone" ]]; then
  set_java "" && ok "Java setting removed"
elif [[ -n "$now$gone" ]]; then
  info "Java: not changed"
else
  info "Java: none. Set it later with: dots private"
fi
