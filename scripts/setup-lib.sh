#!/usr/bin/env bash
# ============================================================================
#  scripts/setup-lib.sh — shell helpers for the few scripts that are still bash:
#  `dotfiles` (installs Rust), scripts/tools, scripts/agent-config, scripts/os-check.sh,
#  scripts/win-golden.sh and the private-words git hook. The setup itself is the Rust `dots`.
#
#  Sourced, never executed.
#
#  Provides: info · ok · warn · skip · step · note · step_end  (logging; quiet unless DOTFILES_VERBOSE=1)
#            have <cmd>                       (is a command on PATH?)
#            is_macos · is_linux · is_windows · is_musl  (which OS; Alpine is musl)
#            linux_prereqs · at_terminal      (Linux: install the system packages dots needs)
#            setting                          (read ~/.config/dotfiles/settings)
#            ensure_rust_bin                  (find or install Rust, build one bin; see the end of this file)
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
    printf '%s•%s %s %s(already set up, skipping)%s\n' "$_c_d" "$_c_0" "$*" "$_c_d" "$_c_0"
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
# stays Linux like any other Linux machine.
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

# is_musl: this Linux uses musl, not glibc (Alpine).
is_musl() { is_linux && compgen -G '/lib/ld-musl-*' >/dev/null 2>&1; }

# The packages that the dotfiles scripts need, for the package
# manager this Linux has. LINUX_PKG is apt-get, dnf, pacman, apk, or empty (unknown:
# then LINUX_PREREQS names commands, and the user installs them).
LINUX_PKG="" LINUX_PREREQS=()
if is_linux; then
  for _p in apt-get dnf pacman apk; do have "$_p" && { LINUX_PKG="$_p"; break; }; done
  # The lists live in scripts/linux-prereqs.txt (the Rust dots reads it too).
  # Plain bash only: no grep or head here.
  while IFS= read -r _p; do
    [[ "$_p" == "${LINUX_PKG:-other}:"* ]] && { read -r -a LINUX_PREREQS <<<"${_p#*:}"; break; }
  done <"${BASH_SOURCE[0]%/*}/linux-prereqs.txt"
  unset _p
fi

# at_terminal: a person can answer (stdin and stderr are a terminal).
at_terminal() { [[ -t 0 && -t 2 ]]; }

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

# Per-machine settings live outside the repo (AGENTS.md "Settings").
DOTFILES_CONF="$HOME/.config/dotfiles"
DOTFILES_SETTINGS="$DOTFILES_CONF/settings"   # NAME="value" lines (settings.example)
OWNER_MD="$DOTFILES_CONF/owner.md"   # the private rules, free text; never in the repo

# setting <NAME>: the value of NAME in the settings file, or nothing. The last NAME
# line wins, and one pair of "" around the value goes. `setting()` in
# rust/agent-config/src/lib.rs and home/.config/zsh/settings.zsh read the file the same way.
setting() {
  [[ -r "$DOTFILES_SETTINGS" ]] || return 0
  awk -v k="$1" 'index($0, k "=") == 1 {
      v = substr($0, length(k) + 2); sub(/[[:space:]]+$/, "", v)
      sub(/^"/, "", v); sub(/"$/, "", v); r = v
    } END { if (r != "") print r }' "$DOTFILES_SETTINGS"
}

# ---- Rust: find or install it, then build one bin of rust/agent-config --------
# The one copy of this code. `dotfiles` (the bin `dots`) and `scripts/tools` (the bin `tools`)
# call ensure_rust_bin. A fresh machine has no Rust: it installs a minimal stable toolchain
# with rustup (into ~/.cargo and ~/.rustup; step 40 adds clippy and rust-analyzer later).

# rust_path: cargo from rustup (~/.cargo/bin) or from Homebrew's keg-only rustup, on PATH.
rust_path() {
  local d
  for d in /home/linuxbrew/.linuxbrew/opt/rustup/bin /usr/local/opt/rustup/bin /opt/homebrew/opt/rustup/bin \
           "${CARGO_HOME:-$HOME/.cargo}/bin"; do
    if [[ -d "$d" && ":$PATH:" != *":$d:"* ]]; then PATH="$d:$PATH"; fi
  done
  export PATH
}

# A cargo that works and a rustc at least as new as rust-version in Cargo.toml (1.89).
# RUSTUP_AUTO_INSTALL=0: with no toolchain on this HOME, fail now, never download one.
rust_ok() {
  RUSTUP_AUTO_INSTALL=0 cargo --version >/dev/null 2>&1 || return 1
  local v; v="$(RUSTUP_AUTO_INSTALL=0 rustc --version 2>/dev/null)" || return 1
  v="${v#rustc }"; v="${v%% *}"
  local major="${v%%.*}" minor="${v#*.}"; minor="${minor%%.*}"
  [[ "$major" =~ ^[0-9]+$ && "$minor" =~ ^[0-9]+$ ]] && (( major > 1 || minor >= 89 ))
}

# install_rust: a minimal stable toolchain with rustup. On Linux the build tools come first
# (scripts/linux-prereqs.txt; Alpine also needs build-base). A subshell, so its pipefail and
# its list changes stay inside.
install_rust() (
  set -o pipefail   # a failed download must fail the install, not feed sh nothing
  if is_linux; then
    [[ "$LINUX_PKG" == apk ]] && LINUX_PREREQS+=(build-base)
    linux_prereqs || return 1
  fi
  printf '»» installing Rust with rustup (about a minute)\n' >&2
  rlog="$(mktemp "${TMPDIR:-/tmp}/dots-rustup.XXXXXX")"
  if is_windows && ! have rustup; then
    # The GNU host brings its own linker, so no Visual Studio Build Tools. It also runs on
    # Windows on Arm (x64 emulation).
    host=x86_64-pc-windows-gnu
    rdir="$(mktemp -d "${TMPDIR:-/tmp}/dots-rustup.XXXXXX")"
    curl --proto '=https' --tlsv1.2 -fsSL -o "$rdir/rustup-init.exe" \
        "https://static.rust-lang.org/rustup/dist/$host/rustup-init.exe" 2>"$rlog" &&
      "$rdir/rustup-init.exe" -y --profile minimal --no-modify-path --default-host "$host" \
        --default-toolchain stable --component llvm-tools >>"$rlog" 2>&1
    rc=$?; rm -rf "$rdir"; (exit $rc)
  elif have rustup; then
    rustup toolchain install stable --profile minimal >"$rlog" 2>&1 && rustup default stable >>"$rlog" 2>&1
  else
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs 2>"$rlog" |
      sh -s -- -y --profile minimal --no-modify-path --default-toolchain stable >>"$rlog" 2>&1
  fi || { printf '!  the Rust install failed (log: %s)\n' "$rlog" >&2; return 1; }
  rm -f "$rlog"
)

# Windows GNU Rust: windows-sys links with raw-dylib, so rustc runs dlltool.exe from PATH.
# The GNU one in rustup needs an assembler that rustup does not ship. LLVM's (llvm-ar from
# llvm-tools, under the name dlltool) needs none. It goes in ~/.cargo/dlltool, first on
# PATH for this build and for the build of the other bins that dots starts.
win_dlltool() {
  local b h d="${CARGO_HOME:-$HOME/.cargo}/dlltool"
  b="$(RUSTUP_AUTO_INSTALL=0 rustc --print sysroot 2>/dev/null)" || return 0
  h="$(RUSTUP_AUTO_INSTALL=0 rustc -vV 2>/dev/null | sed -n 's/^host: //p')"
  b="$(cygpath -u "$b")/lib/rustlib/$h/bin"
  [[ -f "$b/llvm-ar.exe" ]] || rustup component add llvm-tools >/dev/null 2>&1 || return 0
  if [[ ! -f "$d/dlltool.exe" || "$b/llvm-ar.exe" -nt "$d/dlltool.exe" ]]; then
    mkdir -p "$d" && cp "$b"/*.dll "$d/" && cp "$b/llvm-ar.exe" "$d/dlltool.exe" || return 0
  fi
  PATH="$d:$PATH"
}

# ensure_rust_bin <repo> <bin> <label>: make sure target/release/<bin> of rust/agent-config exists
# and is not older than its sources (src, themes, Cargo.toml, Cargo.lock): find Rust (install
# it when it is missing), then build just that bin. Sets RUST_BIN to the file. Returns 1 with
# one `!  <label>: why` line when it cannot. Never touches the other bins.
ensure_rust_bin() {
  local repo="$1" name="$2" label="$3" crate target exe log
  crate="$repo/rust/agent-config"; target="$crate/target"
  exe="$name"; is_windows && exe="$name.exe"
  RUST_BIN="$target/release/$exe"
  rust_path
  if [[ ! -x "$RUST_BIN" ]] || [[ -n "$(find "$crate/src" "$crate/Cargo.toml" "$crate/Cargo.lock" "$crate/themes" \
      -newer "$RUST_BIN" -type f 2>/dev/null | head -1)" ]]; then
    if ! rust_ok; then
      install_rust || { printf '!  %s: Rust could not be installed\n' "$label" >&2; return 1; }
      PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"; export PATH
      rust_ok || { printf '!  %s: no cargo with Rust 1.89 or newer\n' "$label" >&2; return 1; }
    fi
    ! is_windows || win_dlltool
    printf '»» building the Rust %s (the first time takes a few minutes)\n' "$name" >&2
    log="$(mktemp "${TMPDIR:-/tmp}/dots-build.XXXXXX")"
    if RUSTUP_AUTO_INSTALL=0 cargo build --release --quiet --locked --target-dir "$target" \
        --manifest-path "$crate/Cargo.toml" --bin "$name" >"$log" 2>&1; then
      touch "$RUST_BIN"   # cargo leaves it alone when only another bin's source changed
      rm -f "$log"
    else
      printf '!  %s: the Rust build failed (log: %s)\n' "$label" "$log" >&2
      return 1
    fi
  elif is_windows && have rustc; then
    win_dlltool
  fi
  return 0
}
