#!/usr/bin/env bash
# settings.sh: the settings menu (dots settings; dots private opens it too).
# The settings are NAME="value" lines in ~/.config/dotfiles/settings (mode 600,
# never in git). settings.example in the repo lists them all, each with a note.
# The private rules are free text in ~/.config/dotfiles/owner.md. Each change
# saves at once. ./dotfiles runs this on the first run at a terminal, with the
# cursor on Done.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"

LOCAL_ZSH="$REPO/home/.config/zsh/local.zsh"   # ~/.config/zsh links to this folder

# --- Java helpers -------------------------------------------------------------
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
      /usr/lib/jvm/* "$HOME"/.local/share/JetBrains/Toolbox/apps/android-studio/jbr \
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

# java_label <home>: "21.0.5 · Eclipse Adoptium · /Library/Java".
java_label() {
  local major v vendor where
  IFS=$'\t' read -r major v vendor <<<"$(java_info "$1")"
  where="$(java_where "$1")"
  if [[ "$vendor" == "$where" ]]; then vendor=""; fi
  printf '%s%s · %s' "$v" "${vendor:+ · $vendor}" "$where"
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

# --- One editor per setting ---------------------------------------------------
# Each one says what it did. Esc keeps the value as it is.

edit_git_name() {
  local v
  v="$(ask_value "👤 Your name, for git commits" "$(setting GIT_NAME)" "Jane Doe")" || return 0
  if [[ -z "$v" ]]; then warn "a git name can't be empty, so it is not changed"; return 0; fi
  set_setting GIT_NAME "$v" && sync_git
}

edit_git_email() {
  local v
  v="$(ask_value "✉️  Your email, for git commits" "$(setting GIT_EMAIL)" "jane@example.com")" || return 0
  if [[ "$v" != ?*@?*.?* ]]; then warn "that is not an email address, so it is not changed"; return 0; fi
  set_setting GIT_EMAIL "$v" && sync_git
}

# sync_git: write the git name and email to ~/.gitconfig.local when both are set.
sync_git() {
  local n e
  n="$(setting GIT_NAME)" e="$(setting GIT_EMAIL)"
  if [[ -n "$n" && -n "$e" ]]; then git_identity "$n" "$e"; fi
}

edit_profile() {
  local o="🔒 owner: your private rules load" s="🌍 shared: no private rules" now v
  if [[ "$(dotfiles_profile)" == owner ]]; then now="$o"; else now="$s"; fi
  v="$(ask_menu "🧩 Profile" "$now" "$o" "$s")" || return 0
  case "$v" in
    "$o") set_setting PROFILE owner && ok "profile: owner" ;;
    "$s") set_setting PROFILE shared && ok "profile: shared" ;;
  esac
}

# edit_mcp: MCP servers in Cursor are opt-in on every profile (empty means off).
edit_mcp() {
  local on="🔌 on: Cloudflare docs search in Cursor" off="⛔ off: no MCP servers from dots" now v
  if [[ "$(mcp_value)" == on ]]; then now="$on"; else now="$off"; fi
  v="$(ask_menu "🔌 Cursor MCP servers (opt-in)" "$now" "$on" "$off")" || return 0
  case "$v" in
    "$on") set_setting CURSOR_MCP on && ok "Cursor MCP: on" ;;
    "$off") set_setting CURSOR_MCP off && ok "Cursor MCP: off" ;;
  esac
}

# edit_opencode_mcp: MCP servers in Opencode are opt-in on every profile.
edit_opencode_mcp() {
  local on="🔌 on: Cloudflare docs search in Opencode" off="⛔ off: no MCP servers from dots" now v
  if [[ "$(opencode_mcp_value)" == on ]]; then now="$on"; else now="$off"; fi
  v="$(ask_menu "🔌 Opencode MCP servers (opt-in)" "$now" "$on" "$off")" || return 0
  case "$v" in
    "$on") set_setting OPENCODE_MCP on && ok "Opencode MCP: on" ;;
    "$off") set_setting OPENCODE_MCP off && ok "Opencode MCP: off" ;;
  esac
}

# add_java <label> <home>: one line of the Java menu, each Java once.
add_java() {
  local r
  r="$(java_real "$2")"
  if [[ "$seen" == *$'\n'"$r"$'\n'* ]]; then return 0; fi
  seen+="$r"$'\n'
  labels+=("$1")
  homes+=("$2")
}

save_java() {
  set_setting JAVA_HOME "$1" && ok "Java set: $(java_label "$1"). New terminal windows use it."
}

# edit_java: the likely answer comes first: the setting now, else the Java in use
# now, else the newest long-term-support Java. A path can be typed too.
edit_java() {
  local now gone="" used="" pick java="" typed none p h i
  labels=() homes=() seen=$'\n'
  now="$(setting JAVA_HOME)"
  if [[ -n "$now" && -z "$(java_home_of "$now")" ]]; then
    info "JAVA_HOME points to a folder that is gone: $now"
    gone="$now" now=""
  fi
  if [[ -z "$now" ]]; then
    info "JAVA_HOME tells Java tools (Maven, Gradle, editors) which Java to use."
    if [[ -n "${JAVA_HOME:-}" ]]; then used="$(java_home_of "$JAVA_HOME")"; fi
  fi
  if [[ -n "$now" ]]; then add_java "⏭️  Keep it: Java $(java_label "$now")" "$now"; fi
  if [[ -n "$used" ]]; then add_java "☕ Java $(java_label "$used") (in use now)" "$used"; fi
  while IFS= read -r h; do add_java "☕ Java $(java_label "$h")" "$h"; done < <(java_ranked)
  typed="✏️  Type a path"
  if [[ -n "$now$gone" ]]; then none="🗑️  Remove the Java setting"; else none="⏭️  No, skip it"; fi
  if (( ${#homes[@]} == 0 )); then
    info "No Java found in the usual places."
    labels=("$none" "$typed")
  else
    labels+=("$typed" "$none")
  fi
  if [[ -f "$LOCAL_ZSH" ]] && grep -qE '^[^#]*JAVA_HOME=' "$LOCAL_ZSH"; then
    info "Your own lines in local.zsh also set JAVA_HOME. The setting from here runs after them, so it wins."
  fi

  pick="$(ask_one "☕ Which Java should this Mac use? (sets JAVA_HOME)" "${labels[@]}")" || pick=""
  for i in "${!labels[@]}"; do
    if (( i < ${#homes[@]} )) && [[ "${labels[$i]}" == "$pick" ]]; then java="${homes[$i]}"; break; fi
  done
  if [[ -n "$java" && "$java" == "$now" ]]; then
    info "Java: kept ($(java_label "$now"))"
  elif [[ -n "$java" ]]; then
    save_java "$java"
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
      save_java "$h"
    fi
  elif [[ "$pick" == "$none" && -n "$now$gone" ]]; then
    set_setting JAVA_HOME "" && ok "Java setting removed"
  elif [[ -n "$now$gone" ]]; then
    info "Java: not changed"
  else
    info "Java: none set"
  fi
}

# edit_steps: the install.d steps that dots always skips on this Mac.
edit_steps() {
  local f v w out="" now
  local -a all=() picked=() sel=()
  for f in "$REPO"/scripts/install.d/*.sh; do
    if [[ -f "$f" ]]; then all+=("$(basename "$f" .sh)"); fi
  done
  now="$(setting SKIP_STEPS)"
  if fancy; then
    if [[ -n "$now" ]]; then sel=(--selected "$(printf '%s' "$now" | tr -s ' ' ',')"); fi
    v="$(gum choose --no-limit --header "⏩ Steps to skip: x or Tab marks one · Enter saves · Esc keeps" \
      ${sel[@]+"${sel[@]}"} -- ${all[@]+"${all[@]}"})" || { info "steps to skip: not changed"; return 0; }
  else
    info "steps: ${all[*]}"
    v="$(ask_value "⏩ Steps to skip, split by spaces" "$now")" || return 0
  fi
  read -r -a picked <<<"$(printf '%s' "$v" | tr '\n' ' ')"
  for w in ${picked[@]+"${picked[@]}"}; do
    case " ${all[*]} " in
      *" $w "*) out+="${out:+ }$w" ;;
      *) warn "no step named $w, so it is left out" ;;
    esac
  done
  set_setting SKIP_STEPS "$out" && ok "steps to skip: ${out:-none}"
}

# edit_words <NAME> <header> <example> <what>: a list of words, split by spaces.
edit_words() {
  local v
  v="$(ask_value "$2" "$(setting "$1")" "$3")" || return 0
  v="$(printf '%s' "$v" | tr ',' ' ' | tr -s ' ')"
  v="${v# }"; v="${v% }"
  set_setting "$1" "$v" && ok "$4: ${v:-none}"
}

edit_projects() {
  edit_words PRIVATE_PROJECTS "📁 Private project names, split by spaces. The git hooks keep them out of commits." \
    "acme-app side-project" "private projects"
}

# edit_opencode_projects: absolute paths whose .opencode/ mirrors the global Opencode config.
edit_opencode_projects() {
  local v
  v="$(ask_value "📂 Opencode project paths, split by spaces (folders whose .opencode/ mirrors dots; ~ works)" \
      "$(setting OPENCODE_PROJECTS)" "~/code/my-app ~/code/another-one")" || return 0
  v="$(printf '%s' "$v" | tr ',' ' ' | tr -s ' ')"
  v="${v# }"; v="${v% }"
  set_setting OPENCODE_PROJECTS "$v" && ok "Opencode projects: ${v:-none}"
}

edit_words_extra() {
  edit_words PRIVATE_WORDS "🚫 More private words, split by spaces. Your git name and email count already." \
    "client-name nickname" "private words"
}

# edit_rules: the private rules, free text for Claude, Cursor and Opencode. Empty removes them.
edit_rules() {
  local text
  if ! text="$(ask_long "🔒 Private rules for Claude, Cursor and Opencode (Markdown). They stay on this Mac." \
      "$(cat "$OWNER_MD" 2>/dev/null)")"; then
    info "private rules: not changed"
  elif [[ -z "${text//[[:space:]]/}" ]]; then
    rm -f "$OWNER_MD" && info "private rules: none on this Mac"
  else
    mkdir -p "$DOTFILES_CONF" && chmod 700 "$DOTFILES_CONF" &&
      (umask 077 && printf '%s\n' "$text" >"$OWNER_MD") && chmod 600 "$OWNER_MD" &&
      ok "private rules saved ($(lines_of "$OWNER_MD"))"
  fi
}

# --- The menu -----------------------------------------------------------------
# row <emoji> <name> <value>: one menu line. No commas: gum's --selected splits on them.
row() {
  local v="${3//, / }"
  v="${v//,/ }"
  if (( ${#v} > 48 )); then v="${v:0:47}…"; fi
  printf '%s%-18s %s' "$1" "$2" "$v"
}

# value_or <value> <text>: the value, or the text when it is empty.
value_or() { if [[ -n "$1" ]]; then printf '%s' "$1"; else printf '%s' "$2"; fi; }

java_value() {
  local j
  j="$(setting JAVA_HOME)"
  if [[ -z "$j" ]]; then printf 'not set'
  elif [[ -z "$(java_home_of "$j")" ]]; then printf 'gone: %s' "$j"
  else printf 'Java %s' "$(java_label "$j")"; fi
}

# mcp_value: on or off. Like agent-config.py: 1, on, yes or true is on.
mcp_value() {
  case "$(setting CURSOR_MCP | tr '[:upper:]' '[:lower:]')" in
    1|on|yes|true) printf 'on' ;;
    *) printf 'off' ;;
  esac
}

# opencode_mcp_value: on or off. DOTFILES_MCP wins over the setting.
opencode_mcp_value() {
  case "$(setting OPENCODE_MCP | tr '[:upper:]' '[:lower:]')" in
    1|on|yes|true) printf 'on' ;;
    *) printf 'off' ;;
  esac
}

# opencode_projects_value: the configured project paths, or "none".
opencode_projects_value() {
  local v
  v="$(setting OPENCODE_PROJECTS)"
  if [[ -z "$v" ]]; then printf 'none'
  else printf '%s' "$v"; fi
}

# lines_of <file>: "1 line" or "N lines".
lines_of() {
  local n
  n="$(wc -l <"$1" | tr -d ' ')"
  if [[ "$n" == 1 ]]; then printf '1 line'; else printf '%s lines' "$n"; fi
}

rules_value() {
  if [[ -s "$OWNER_MD" ]]; then lines_of "$OWNER_MD"; else printf 'none'; fi
}

step "Settings (they stay on this Mac, never in git)"
info "File: ~/.config/dotfiles/settings. Each change saves at once."
done_row="✅ Done"
keys=(edit_git_name edit_git_email edit_profile edit_mcp edit_opencode_mcp edit_java edit_steps edit_projects
  edit_opencode_projects edit_words_extra edit_rules)
start="" last=""
if [[ -n "${DOTFILES_FIRST_RUN:-}" ]]; then start="$done_row"; fi
while :; do
  rows=(
    "$(row "👤 " "Git name" "$(value_or "$(setting GIT_NAME)" "not set")")"
    "$(row "📧 " "Git email" "$(value_or "$(setting GIT_EMAIL)" "not set")")"
    "$(row "🧩 " "Profile" "$(dotfiles_profile)")"
    "$(row "🔌 " "Cursor MCP" "$(mcp_value)")"
    "$(row "🔌 " "Opencode MCP" "$(opencode_mcp_value)")"
    "$(row "☕ " "Java" "$(java_value)")"
    "$(row "⏩ " "Steps to skip" "$(value_or "$(setting SKIP_STEPS)" "none")")"
    "$(row "📁 " "Private projects" "$(value_or "$(setting PRIVATE_PROJECTS)" "none")")"
    "$(row "📂 " "Opencode projects" "$(opencode_projects_value)")"
    "$(row "🚫 " "Private words" "$(value_or "$(setting PRIVATE_WORDS)" "none")")"
    "$(row "🔒 " "Private rules" "$(rules_value)")"
  )
  if [[ -n "$last" ]]; then start="${rows[$last]}"; fi
  pick="$(ask_menu "⚙️  Pick a setting to change, or Done (↑ ↓, Enter)" "$start" "${rows[@]}" "$done_row")" \
    || break
  if [[ -z "$pick" || "$pick" == "$done_row" ]]; then break; fi
  last=""
  for i in "${!rows[@]}"; do
    if [[ "${rows[$i]}" == "$pick" ]]; then last=$i; break; fi
  done
  if [[ -z "$last" ]]; then break; fi
  "${keys[$last]}"
done
ok "settings: done"
