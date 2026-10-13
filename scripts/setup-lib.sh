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
#            ask_text · ask_email · ask_one · ask_long · spin
#            link_item <src> <dst>             (symlink, backing up a conflict)
#            dotfiles_profile                  (owner or shared)
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
    # A fixed width: gum input can crash when the terminal reports no size.
    if ! fancy || ! v="$(gum input --width 60 --header "$label" --placeholder "$hint" --prompt "› ")"; then
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

# ask_one <header> <label>...: pick one label with ↑ ↓ and Enter; prints it.
ask_one() {
  local header="$1" i=0 a="" item; shift
  if fancy && gum choose --header "$header" -- "$@"; then return 0; fi
  printf '%s\n' "$header" >&2
  for item in "$@"; do i=$((i + 1)); printf '  %d) %s\n' "$i" "$item" >&2; done
  read -r -p "  Number [1]: " a || return 1
  [[ "$a" =~ ^[0-9]+$ && "$a" -ge 1 && "$a" -le $# ]] || a=1
  i=0
  for item in "$@"; do i=$((i + 1)); [[ $i -eq $a ]] && printf '%s\n' "$item"; done
  return 0
}

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

# Per-machine switches live outside the repo (AGENTS.md "Profiles").
DOTFILES_CONF="$HOME/.config/dotfiles"
OWNER_MD="$DOTFILES_CONF/owner.md"   # the owner's private rules; never in the repo

# dotfiles_profile: print "owner" or "shared" (AGENTS.md "Profiles"). Order:
# $DOTFILES_PROFILE, then $DOTFILES_CONF/profile, then "owner" when the owner
# overlay $OWNER_MD exists (the answer is saved to the profile file), else
# "shared". scripts/agent-config.py default_profile follows the same order.
dotfiles_profile() {
  local f="$DOTFILES_CONF/profile" p="${DOTFILES_PROFILE:-}"
  if [[ -z "$p" && -r "$f" ]]; then p="$(tr -d '[:space:]' < "$f")"; fi
  if [[ -z "$p" && -f "$OWNER_MD" ]]; then
    p=owner
    { printf 'owner\n' > "$f"; } 2>/dev/null
  fi
  case "$p" in owner|shared) ;; *) p=shared ;; esac
  printf '%s\n' "$p"
}
