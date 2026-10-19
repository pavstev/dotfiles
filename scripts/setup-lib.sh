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
#  Provides: info · ok · warn · skip · step  (logging; quiet unless DOTFILES_VERBOSE=1)
#            uptodate · ok_if_changed · note · step_end  (the one-line-per-step output)
#            have <cmd>                       (is a command on PATH?)
#            is_macos · is_linux · only_macos (which OS; skip a Mac-only step)
#            is_musl                          (Alpine: no Homebrew)
#            linux_prereqs · brewfile_for_os  (Linux: system packages, Brewfile without Mac lines)
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

# Output. A run prints one short line per step. DOTFILES_VERBOSE=1 (or dots -v) prints it all.
# A step header waits until the step prints something. A step that only finds things
# up to date ends as one line: "• Title: 5 up to date". Warnings and errors always print.
verbose() { [[ -n "${DOTFILES_VERBOSE:-}" && "${DOTFILES_VERBOSE}" != 0 ]]; }

_STEP_TITLE="" _STEP_SHOWN="" _STEP_SKIPS=0
_step_head() {   # print the waiting step header, once
  if [[ -n "$_STEP_TITLE" && -z "$_STEP_SHOWN" ]]; then
    printf '\n%s▸ %s%s\n' "$_c_a" "$_STEP_TITLE" "$_c_0"
    _STEP_SHOWN=1
  fi
}
# step_end: close the step in progress with its count line. Runs on its own at exit.
step_end() {
  if [[ -n "$_STEP_TITLE" && $_STEP_SKIPS -gt 0 ]]; then
    if [[ "$_STEP_SHOWN" == 1 ]]; then
      printf '%s•%s %s up to date\n' "$_c_d" "$_c_0" "$_STEP_SKIPS"
    elif [[ $_STEP_SKIPS -eq 1 ]]; then
      printf '%s•%s %s: up to date\n' "$_c_d" "$_c_0" "$_STEP_TITLE"
    else
      printf '%s•%s %s: %s up to date\n' "$_c_d" "$_c_0" "$_STEP_TITLE" "$_STEP_SKIPS"
    fi
  fi
  _STEP_TITLE="" _STEP_SHOWN="" _STEP_SKIPS=0
}
step() {
  step_end
  if verbose; then
    printf '\n%s▸ %s%s\n' "$_c_a" "$*" "$_c_0"
    _STEP_SHOWN=1
  else
    trap step_end EXIT
  fi
  _STEP_TITLE="$*"
}
info() { _step_head; printf '%s»»%s %s\n'  "$_c_b" "$_c_0" "$*"; }
ok()   { _step_head; printf '%s✓%s %s\n'   "$_c_g" "$_c_0" "$*"; }
# warn also adds a line to DOTFILES_WARN_FILE (./dotfiles sets it), so the last line of a
# run can count the warnings of every step.
warn() {
  _step_head; printf '%s!%s  %s\n'  "$_c_y" "$_c_0" "$*" >&2
  if [[ -n "${DOTFILES_WARN_FILE:-}" ]]; then printf '%s\n' "$*" >>"$DOTFILES_WARN_FILE" 2>/dev/null || true; fi
}
# skip <what>: it was already there. Counted in a step, printed in verbose mode.
skip() {
  if verbose || [[ -z "$_STEP_TITLE" ]]; then
    printf '%s•%s %s %s(already set up — skipping)%s\n' "$_c_d" "$_c_0" "$*" "$_c_d" "$_c_0"
  else
    _STEP_SKIPS=$((_STEP_SKIPS + 1))
  fi
}
# uptodate <msg>: the step found nothing to do. A line in verbose mode, a count otherwise.
uptodate() { if verbose; then ok "$*"; else skip "$*"; fi; }
# ok_if_changed <msg>: a closing line, only when the step did or said something before.
ok_if_changed() { if verbose || [[ -n "$_STEP_SHOWN" || -z "$_STEP_TITLE" ]]; then ok "$*"; fi; }
# note <msg>: the whole step in one line ("• Title: msg"), when there is nothing to do.
note() {
  if verbose || [[ -z "$_STEP_TITLE" ]]; then info "$*"; return; fi
  printf '%s•%s %s: %s\n' "$_c_d" "$_c_0" "$_STEP_TITLE" "$*"
  _STEP_SHOWN=2
}

# have <cmd> — true if <cmd> resolves on PATH. The basis of "skip if installed".
have() { command -v "$1" >/dev/null 2>&1; }

# is_macos / is_linux / is_windows: which OS this is. Every OS-only branch uses these.
# WSL reports Linux via uname, so is_windows is Git Bash / MSYS / Cygwin only — WSL
# stays Linux and gets Homebrew on Linux like any other Linux machine.
is_macos() { [[ "$(uname -s)" == Darwin ]]; }
is_linux() { [[ "$(uname -s)" == Linux ]]; }
is_windows() {
  case "$(uname -s 2>/dev/null)" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) return 0 ;;
  esac
  case "${OSTYPE:-}" in
    msys*|cygwin*|win32) return 0 ;;
  esac
  return 1
}

# Git Bash `ln -s` makes COPIES unless MSYS asks for native links. A copy never follows
# the repo, so Windows must make real links (they need Developer Mode on).
is_windows && export MSYS=winsymlinks:nativestrict

# windows_links_ready: true when this shell can make a real symlink (Windows only).
windows_links_ready() {
  local d; d="$(mktemp -d "${TMPDIR:-/tmp}/dots-link.XXXXXX")" || return 1
  : >"$d/a"
  if ln -s "$d/a" "$d/b" 2>/dev/null && [[ -L "$d/b" ]]; then rm -rf "$d"; return 0; fi
  rm -rf "$d"; return 1
}

# winpath <posix-path>: a Windows path (C:\Users\...) for the POSIX path, for
# Windows apps that cannot read /c/Users-style paths. Needs cygpath (Git Bash
# ships it); falls back to the input unchanged.
winpath() {
  if have cygpath; then cygpath -w -- "$1" 2>/dev/null || printf '%s\n' "$1"; else printf '%s\n' "$1"; fi
}

# only_macos <what>: in a step, stop here with a clear line when this is not a Mac.
only_macos() { is_macos || { if verbose; then info "$1: macOS only, skipping"; fi; exit 0; }; }

# brew_env — make `brew` (and brew-installed tools) available to a script that
# runs in a bare, non-interactive environment. No-op if brew is absent. Homebrew
# lives in /opt/homebrew (Apple Silicon), /usr/local (Intel Mac) or
# /home/linuxbrew/.linuxbrew (Linux; ~/.linuxbrew without root).
brew_env() {
  local b r
  for b in /opt/homebrew/bin/brew /usr/local/bin/brew \
           /home/linuxbrew/.linuxbrew/bin/brew "$HOME/.linuxbrew/bin/brew"; do
    [[ -x "$b" ]] || continue
    eval "$("$b" shellenv)"
    # Homebrew's rustup is keg-only: cargo and rustc are in opt/rustup/bin (zsh adds it too).
    r="$HOMEBREW_PREFIX/opt/rustup/bin"
    if [[ -d "$r" && ":$PATH:" != *":$r:"* ]]; then PATH="$r:$PATH"; fi
    return 0
  done
}

# is_musl: this Linux uses musl, not glibc (Alpine). Homebrew on Linux needs glibc.
is_musl() { is_linux && compgen -G '/lib/ld-musl-*' >/dev/null 2>&1; }

# The packages that Homebrew on Linux and the dotfiles scripts need, for the package
# manager this Linux has. LINUX_PKG is apt-get, dnf, pacman, apk, or empty (unknown:
# then LINUX_PREREQS names commands, and the user installs them).
LINUX_PKG="" LINUX_PREREQS=()
if is_linux; then
  for _p in apt-get dnf pacman apk; do have "$_p" && { LINUX_PKG="$_p"; break; }; done
  unset _p
  case "$LINUX_PKG" in
    apt-get) LINUX_PREREQS=(build-essential ca-certificates curl file git procps python3 zsh) ;;
    dnf) LINUX_PREREQS=(gcc gcc-c++ make ca-certificates curl file git procps-ng python3 zsh) ;;
    pacman) LINUX_PREREQS=(base-devel ca-certificates curl file git procps-ng python zsh) ;;
    apk) LINUX_PREREQS=(bash ca-certificates curl file git python3 zsh) ;;
    *) LINUX_PREREQS=(cc make curl file git ps python3 zsh) ;;
  esac
fi

# linux_prereqs_missing: the packages from LINUX_PREREQS that are not installed.
linux_prereqs_missing() {
  local p
  for p in "${LINUX_PREREQS[@]}"; do
    case "$LINUX_PKG" in
      apt-get) dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q 'install ok installed' ;;
      dnf) rpm -q "$p" >/dev/null 2>&1 ;;
      pacman) pacman -Q "$p" >/dev/null 2>&1 ;;
      apk) apk info -e "$p" >/dev/null 2>&1 ;;
      *) have "$p" ;;
    esac || printf '%s\n' "$p"
  done
}

# linux_prereqs: install what linux_prereqs_missing lists, with apt, dnf, pacman or apk.
# Quiet when nothing is missing. Exit 0 = all there.
linux_prereqs() {
  is_linux || return 0
  local missing sudo_cmd="" name="${LINUX_PKG%-get}"
  missing="$(linux_prereqs_missing | tr '\n' ' ')"
  missing="${missing% }"
  [[ -z "$missing" ]] && return 0
  if [[ -z "$LINUX_PKG" ]]; then
    warn "no apt, dnf, pacman or apk here. Install these commands yourself, then run dots again: $missing"
    return 1
  fi
  if [[ "$(id -u)" -ne 0 ]]; then
    have sudo || { warn "sudo is missing. As root, install with $name: $missing"; return 1; }
    sudo_cmd="sudo"
    if at_terminal && ! sudo -n true 2>/dev/null; then info "$name needs your password once."; fi
  fi
  info "installing with $name: $missing"
  local log rc=0; log="$(mktemp "${TMPDIR:-/tmp}/dots-pkg.XXXXXX")"
  # shellcheck disable=SC2086  # $missing is a list of package names
  case "$LINUX_PKG" in
    apt-get)
      $sudo_cmd env DEBIAN_FRONTEND=noninteractive apt-get update -qq >"$log" 2>&1 &&
        $sudo_cmd env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends $missing >>"$log" 2>&1 || rc=1 ;;
    dnf) $sudo_cmd dnf install -y -q $missing >"$log" 2>&1 || rc=1 ;;
    pacman) $sudo_cmd pacman -S --needed --noconfirm $missing >"$log" 2>&1 || rc=1 ;;
    apk) $sudo_cmd apk add -q --no-cache $missing >"$log" 2>&1 || rc=1 ;;
  esac
  if ((rc == 0)); then
    rm -f "$log"
    ok "$name packages installed"
  else
    warn "$name could not install: $missing"
    tail -5 "$log" >&2
    rm -f "$log"
    return 1
  fi
}

# ensure_brew: install Homebrew when it is missing, then load it. At a terminal
# it asks for the password first (the installer needs sudo). Exit 0 = brew works.
# On Linux the prerequisites go in first. Homebrew is not for Windows or musl.
ensure_brew() {
  is_windows && { warn "Homebrew is not for Windows — WinGet installs the tools instead"; return 0; }
  brew_env
  have brew && return 0
  is_musl && return 1   # Homebrew needs glibc; step 10 says so
  linux_prereqs || warn "going on without the prerequisites. The Homebrew installer may complain."
  if at_terminal; then
    if is_macos; then info "Homebrew needs your Mac password once."; else info "Homebrew needs your password once."; fi
    sudo -v || return 1
  fi
  NONINTERACTIVE=1 /bin/bash -c \
    "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" || return 1
  brew_env
  have brew
}

# brewfile_for_os <file>: the Brewfile to give brew bundle. On a Mac, the file itself.
# On Linux, a temp copy without the lines Linux cannot use: cask, mas, tap, vscode
# (casks and the App Store are macOS only; extensions are installed by the steps that
# know the editor). The caller deletes the temp file.
brewfile_for_os() {
  local f="$1" t
  if is_macos; then printf '%s\n' "$f"; return 0; fi
  t="$(mktemp "${TMPDIR:-/tmp}/Brewfile.XXXXXX")" || return 1
  grep -vE '^[[:space:]]*(cask|mas|tap|vscode)[[:space:]]|^[[:space:]]*brew[[:space:]]+"mas"' "$f" >"$t"
  printf '%s\n' "$t"
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
LINK_COUNT=0   # links made since the last count reset; DOTFILES_LINK_QUIET=1 prints a total, not each
link_item() {
  local s="$1" d="$2"
  [[ -e "$s" ]] || return 0
  if [[ -L "$d" && "$(readlink "$d")" == "$s" ]]; then
    if ! verbose && [[ -n "$_STEP_TITLE" ]]; then _STEP_SKIPS=$((_STEP_SKIPS + 1)); fi
    return 0
  fi
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
    LINK_COUNT=$((LINK_COUNT + 1))
    [[ -n "${DOTFILES_LINK_QUIET:-}" ]] || ok "linked $(basename "$d")"
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
    if verbose; then skip "git identity ($1 <$2>)"; fi
  elif git config -f "$f" user.name "$1" && git config -f "$f" user.email "$2"; then
    ok "git commits use: $1 <$2>"
  else
    warn "couldn't write $f"
  fi
}

# windows_gitconfig: on Windows the repo's ~/.gitconfig is not linked. It sets delta as the
# pager and VS Code as the editor, and a Windows machine has neither. The identity
# still works: ~/.gitconfig includes ~/.gitconfig.local. A repo ~/.gitconfig left by an
# earlier run (a link or a copy) goes, and the newest backup comes back.
windows_gitconfig() {
  local g="$HOME/.gitconfig" b
  if [[ -L "$g" ]] || grep -q 'symlinked from this repo' "$g" 2>/dev/null; then
    rm -f "$g"
    b="$(ls -t "$g".bak-* 2>/dev/null | head -1)"
    if [[ -n "$b" ]]; then mv "$b" "$g" && info "restored your own .gitconfig"; fi
  fi
  if ! git config --global --get-all include.path 2>/dev/null | grep -q 'gitconfig.local'; then
    git config --global --add include.path "~/.gitconfig.local"
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
