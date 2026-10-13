#!/usr/bin/env bash
# ============================================================================
#  scripts/setup-lib.sh — shared helpers for the scripts/install.d/ steps.
#
#  Sourced, never executed. Gives every install step the same warm, skip-aware
#  voice: already-installed tools are *detected and skipped*, not reinstalled,
#  and the output reads as a coherent story across the whole run.
#
#  Usage — from a scripts/install.d/*.sh step (run by ./dotfiles, which
#  exports REPO before invoking each one):
#      . "$REPO/scripts/setup-lib.sh"
#
#  Provides: info · ok · warn · skip · step  (logging)
#            have <cmd>                       (is a command on PATH?)
#            brew_env · ensure_brew           (load or install Homebrew)
#            at_terminal · fancy · banner     (guided setup with gum menus)
#            ask_text · ask_email · ask_value · ask_one · ask_menu · ask_long · spin
#            link_item <src> <dst>             (symlink, backing up a conflict)
#            setting · set_setting             (~/.config/dotfiles/settings)
#            git_identity · dotfiles_profile   (owner or shared)
# ============================================================================

# Copper-on-stone palette, but only when stdout is a real TTY and the caller
# hasn't asked for plain output (NO_COLOR). Keeps logs clean in CI / pipes.
if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
  _c_b=$'\033[38;5;73m'    # teal     — info / in-progress
  _c_g=$'\033[38;5;108m'   # sage     — success
  _c_y=$'\033[38;5;179m'   # amber    — warning
  _c_d=$'\033[38;5;245m'   # stone    — skipped / dim
  _c_a=$'\033[1;38;5;173m' # copper   — section heading (the signature colour)
  _c_0=$'\033[0m'
else
  _c_b='' _c_g='' _c_y='' _c_d='' _c_a='' _c_0=''
fi

info() { printf '%s»»%s %s\n'  "$_c_b" "$_c_0" "$*"; }
ok()   { printf '%s✓%s %s\n'   "$_c_g" "$_c_0" "$*"; }
warn() { printf '%s!%s  %s\n'  "$_c_y" "$_c_0" "$*" >&2; }
skip() { printf '%s•%s %s %s(already set up — skipping)%s\n' "$_c_d" "$_c_0" "$*" "$_c_d" "$_c_0"; }
step() { printf '\n%s▸ %s%s\n' "$_c_a" "$*" "$_c_0"; }

# have <cmd> — true if <cmd> resolves on PATH. The basis of "skip if installed".
have() { command -v "$1" >/dev/null 2>&1; }

# brew_env — make `brew` (and brew-installed tools) available to a script that
# runs in a bare, non-interactive environment. No-op if brew is absent.
brew_env() {
  if   [[ -x /opt/homebrew/bin/brew ]]; then eval "$(/opt/homebrew/bin/brew shellenv)"
  elif [[ -x /usr/local/bin/brew   ]]; then eval "$(/usr/local/bin/brew shellenv)"
  fi
}

# ensure_brew: install Homebrew when it is missing, then load it. At a terminal
# it asks for the Mac password first (the installer needs sudo). Exit 0 = brew works.
ensure_brew() {
  brew_env
  have brew && return 0
  if at_terminal; then
    info "Homebrew needs your Mac password once."
    sudo -v || return 1
  fi
  NONINTERACTIVE=1 /bin/bash -c \
    "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" || return 1
  brew_env
  have brew
}

# ---- Guided prompts ---------------------------------------------------------
# A person at a terminal gets gum menus (charmbracelet/gum, in the Brewfile);
# anything else (a pipe, a test) gets plain `read`. GUM_* sets the copper palette.
export GUM_INPUT_PROMPT_FOREGROUND=173 GUM_INPUT_CURSOR_FOREGROUND=173 \
  GUM_INPUT_HEADER_FOREGROUND=245 GUM_INPUT_PLACEHOLDER_FOREGROUND=240 \
  GUM_CHOOSE_CURSOR_FOREGROUND=173 GUM_CHOOSE_SELECTED_FOREGROUND=173 \
  GUM_CHOOSE_HEADER_FOREGROUND=245 GUM_CONFIRM_PROMPT_FOREGROUND=173 \
  GUM_CONFIRM_SELECTED_BACKGROUND=173 GUM_SPIN_SPINNER_FOREGROUND=173 \
  GUM_WRITE_HEADER_FOREGROUND=245 GUM_WRITE_CURSOR_FOREGROUND=173 \
  GUM_WRITE_PLACEHOLDER_FOREGROUND=240

# A person can answer: stdin and stderr are the terminal. Not stdout: gum draws on
# stderr, so "$(ask_text ...)" still shows its menu.
at_terminal() { [[ -t 0 && -t 2 ]]; }
fancy() { at_terminal && have gum; }    # ... and gum can draw the menus

# banner <title> [line...]: a copper box, or plain lines without gum.
banner() {
  local title="$1"; shift
  if fancy; then
    gum style --border rounded --border-foreground 173 --padding "0 2" --margin "1 0" \
      "$(gum style --bold --foreground 173 "$title")" "$@"
  else
    printf '\n%s\n' "$title"
    if [[ $# -gt 0 ]]; then printf '  %s\n' "$@"; fi
  fi
}

# ask_text <label> [example] [empty-ok]: one line of text. Asked again while it is
# empty, unless the third word is empty-ok.
ask_text() {
  local label="$1" hint="${2:-}" empty="${3:-}" v=""
  while :; do
    # A fixed width: gum input can crash when the terminal reports no size. No
    # length limit: gum cuts text at 400 characters by default.
    if ! fancy || ! v="$(gum input --width 60 --char-limit 0 --header "$label" --placeholder "$hint" --prompt "› ")"; then
      read -r -p "$label: " v || return 1
    fi
    v="${v#"${v%%[![:space:]]*}"}"; v="${v%"${v##*[![:space:]]}"}"
    if [[ -n "$v" || "$empty" == empty-ok ]]; then break; fi
  done
  printf '%s\n' "$v"
}

# ask_email <label> [example]: like ask_text, but it must look like an email.
ask_email() {
  local v
  while v="$(ask_text "$@")"; do
    if [[ "$v" == ?*@?*.?* ]]; then printf '%s\n' "$v"; return 0; fi
    info "that is not an email address, try again"
  done
  return 1
}

# ask_value <label> <now> [example]: one line, with the value now filled in. Plain
# fallback: Enter keeps it, a lone - empties it. Esc keeps it too (exit 1).
ask_value() {
  local label="$1" now="$2" hint="${3:-}" v
  if fancy; then
    v="$(gum input --width 60 --char-limit 0 --header "$label" --value "$now" --placeholder "$hint" --prompt "› ")" \
      || return 1
  else
    read -r -p "$label [$now]: " v || return 1
    if [[ -z "$v" ]]; then v="$now"; elif [[ "$v" == - ]]; then v=""; fi
  fi
  v="${v#"${v%%[![:space:]]*}"}"; v="${v%"${v##*[![:space:]]}"}"
  printf '%s\n' "$v"
}

# ask_menu <header> <start> <label>...: pick one label with ↑ ↓ and Enter; prints
# it. The cursor starts on the label <start> (empty: the first). No commas in
# <start>: gum splits it on them. Esc or Ctrl+C: no answer (exit 1). The numbered
# list is for a Mac without gum, or a gum that fails.
ask_menu() {
  local header="$1" start="$2" i=0 n=1 a="" item; shift 2
  local -a sel=()
  if [[ -n "$start" ]]; then sel=(--selected "$start"); fi
  if fancy; then
    gum choose --header "$header" ${sel[@]+"${sel[@]}"} -- "$@" && return 0
    case $? in 1|130) return 1 ;; esac
  fi
  printf '%s\n' "$header" >&2
  for item in "$@"; do
    i=$((i + 1))
    if [[ "$item" == "$start" ]]; then n=$i; fi
    printf '  %d) %s\n' "$i" "$item" >&2
  done
  read -r -p "  Number [$n]: " a || return 1
  [[ "$a" =~ ^[0-9]+$ && "$a" -ge 1 && "$a" -le $# ]] || a=$n
  i=0
  for item in "$@"; do i=$((i + 1)); [[ $i -eq $a ]] && printf '%s\n' "$item"; done
  return 0
}

# ask_one <header> <label>...: ask_menu with the cursor on the first label.
ask_one() { local header="$1"; shift; ask_menu "$header" "" "$@"; }

# ask_long <header> [text]: several lines of text in a box (gum write). Pasting
# works; Enter saves, Ctrl+J adds a line, Esc cancels (exit 1). Plain fallback:
# lines until one that holds only a dot; an empty answer keeps [text].
ask_long() {
  local header="$1" value="${2:-}" line out=""
  if fancy; then
    gum write --header "$header"$'\n'"Enter saves · Ctrl+J adds a line · Esc cancels" \
      --value "$value" --placeholder "Paste or type here." --width 80 --height 12
    return
  fi
  printf '%s\nPaste or type. End with a line that holds only a dot.\n' "$header" >&2
  while IFS= read -r line; do
    [[ "$line" == . ]] && break
    out+="$line"$'\n'
  done
  printf '%s' "${out:-$value}"
}

# spin <title> <command...>: run a command quietly, with a spinner under gum.
spin() {
  local title="$1"; shift
  if fancy; then gum spin --title "$title" -- "$@"; else "$@" >/dev/null 2>&1; fi
}

# link_item <src> <dst> — symlink dst -> src, idempotent and non-destructive:
# already-correct links are left alone; a pre-existing real file/dir/symlink
# elsewhere is backed up to <dst>.bak-<timestamp> before linking.
link_item() {
  local s="$1" d="$2"
  [[ -e "$s" ]] || return 0
  if [[ -L "$d" && "$(readlink "$d")" == "$s" ]]; then return 0; fi
  if [[ -e "$d" || -L "$d" ]]; then
    local ts; ts="$(date +%Y%m%d-%H%M%S)"
    if mv "$d" "$d.bak-$ts" 2>/dev/null; then
      warn "backed up existing $(basename "$d") → $d.bak-$ts"
    else
      warn "couldn't back up $d — leaving it in place, skipping link"
      return 0
    fi
  fi
  mkdir -p "$(dirname "$d")" 2>/dev/null
  if ln -s "$s" "$d" 2>/dev/null; then
    ok "linked $(basename "$d")"
  else
    warn "couldn't link $d → $s"
  fi
}

# Per-machine settings live outside the repo (AGENTS.md "Settings").
DOTFILES_CONF="$HOME/.config/dotfiles"
DOTFILES_SETTINGS="$DOTFILES_CONF/settings"   # NAME="value" lines (settings.example)
OWNER_MD="$DOTFILES_CONF/owner.md"   # the private rules, free text; never in the repo

# setting <NAME>: the value of NAME in the settings file, or nothing. The last NAME
# line wins, and one pair of "" around the value goes. scripts/agent-config.py
# setting() and home/.config/zsh/settings.zsh read the file the same way.
setting() {
  [[ -r "$DOTFILES_SETTINGS" ]] || return 0
  awk -v k="$1" 'index($0, k "=") == 1 {
      v = substr($0, length(k) + 2); sub(/[[:space:]]+$/, "", v)
      sub(/^"/, "", v); sub(/"$/, "", v); r = v
    } END { if (r != "") print r }' "$DOTFILES_SETTINGS"
}

# set_setting <NAME> <value>: save NAME="value" in the settings file (mode 600).
# It replaces the first NAME line and drops later ones, or adds the line at the
# end. A new file starts as a copy of settings.example. Other lines stay. When the
# file is a link, the file it points to changes and the link stays.
set_setting() (
  umask 077
  case "$2" in
    *[\"\\\$\`]*|*$'\n'*) warn "$1 not saved: a value can't hold \" \\ \$ \` or a new line"; exit 1 ;;
  esac
  mkdir -p "$DOTFILES_CONF" && chmod 700 "$DOTFILES_CONF" || exit 1
  dest="$DOTFILES_SETTINGS"
  for i in 1 2 3 4 5 6 7 8; do
    [[ -L "$dest" ]] || break
    l="$(readlink "$dest")" || exit 1
    case "$l" in /*) dest="$l" ;; *) dest="$(dirname "$dest")/$l" ;; esac
  done
  # The new file goes next to the old one, so mv replaces it in one step.
  tmp="$(mktemp "$(dirname "$dest")/.settings.XXXXXX")" || exit 1
  if [[ -f "$dest" ]]; then src="$dest"
  elif [[ -f "${REPO:-}/settings.example" ]]; then src="$REPO/settings.example"
  else src=/dev/null; fi
  if awk -v k="$1" -v line="$1=\"$2\"" '
      index($0, k "=") == 1 { if (!d) print line; d = 1; next }
      { print } END { if (!d) print line }' "$src" >"$tmp" &&
     mv -f "$tmp" "$dest"; then
    chmod 600 "$dest"
  else
    rm -f "$tmp"
    exit 1
  fi
)

# git_identity <name> <email>: write them to ~/.gitconfig.local when they differ.
git_identity() {
  local f="$HOME/.gitconfig.local"
  if [[ "$(git config -f "$f" user.name 2>/dev/null)" == "$1" &&
        "$(git config -f "$f" user.email 2>/dev/null)" == "$2" ]]; then
    skip "git identity ($1 <$2>)"
  elif git config -f "$f" user.name "$1" && git config -f "$f" user.email "$2"; then
    ok "git commits use: $1 <$2>"
  else
    warn "couldn't write $f"
  fi
}

# dotfiles_profile: print "owner" or "shared" (AGENTS.md "Profiles"). Order:
# $DOTFILES_PROFILE, then PROFILE in the settings, then "owner" when the private
# rules $OWNER_MD exist (saved as PROFILE), else "shared".
# scripts/agent-config.py default_profile follows the same order.
dotfiles_profile() {
  local p="${DOTFILES_PROFILE:-}"
  [[ -n "$p" ]] || p="$(setting PROFILE)"
  if [[ -z "$p" && -f "$OWNER_MD" ]]; then
    p=owner
    set_setting PROFILE owner >/dev/null 2>&1 || true
  fi
  case "$p" in owner|shared) ;; *) p=shared ;; esac
  printf '%s\n' "$p"
}
