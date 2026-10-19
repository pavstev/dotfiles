#!/usr/bin/env bash
# fetch.sh <youtube url or id>: print the path of a timestamped markdown transcript.
# Cache: ~/.cache/yt-learn/<video id>.md (a rerun is free).
# Tool: the yt_transcribe Rust bin in this repo (rust/agent-config). It is built on first
# use and again when its source is newer. It needs yt-dlp; the Whisper fallback also needs
# ffmpeg and whisper-cli (it says how to install each one when it is missing).
set -euo pipefail

url="${1:-}"
[ -n "$url" ] || { echo "usage: fetch.sh <youtube url>" >&2; exit 2; }

# The skill folder is a symlink into the repo; resolve it to find the crate.
HERE="$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO="$(cd "$HERE/../../../.." && pwd)"
[ -f "$REPO/rust/agent-config/Cargo.toml" ] || REPO="${DOTFILES_REPO:-$HOME/dotfiles}"
CRATE="$REPO/rust/agent-config"
[ -f "$CRATE/Cargo.toml" ] || { echo "error: dotfiles repo not found at $REPO. Set DOTFILES_REPO." >&2; exit 1; }

EXE=yt_transcribe
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) EXE=yt_transcribe.exe ;; esac
TARGET="$CRATE/target"
BIN="$TARGET/release/$EXE"

stale() {
  [ ! -x "$BIN" ] && return 0
  [ -n "$(find "$CRATE/src/bin/yt_transcribe.rs" "$CRATE/Cargo.toml" "$CRATE/Cargo.lock" \
    -newer "$BIN" -type f 2>/dev/null | head -1)" ]
}

if stale; then
  command -v cargo >/dev/null 2>&1 || {
    echo "error: cargo is not installed, and yt_transcribe must be built once. Install Rust: https://rustup.rs (or: brew install rustup)." >&2
    exit 1
  }
  echo "yt-learn: building yt_transcribe (first run takes a minute)" >&2
  RUSTUP_AUTO_INSTALL=0 cargo build --release --quiet --locked --target-dir "$TARGET" \
    --manifest-path "$CRATE/Cargo.toml" --bin yt_transcribe >&2 || {
    echo "error: building yt_transcribe failed" >&2
    exit 1
  }
  touch "$BIN"
fi

# One parser for every link form: --print-id (pure, no network).
id="$("$BIN" --print-id "$url")" || exit 1

dir="$HOME/.cache/yt-learn"
out="$dir/$id.md"
mkdir -p "$dir"

if [ -s "$out" ]; then
  echo "$out"
  exit 0
fi

tmp="$(mktemp "$dir/.$id.XXXXXX")"
err="$(mktemp)"
trap 'rm -f "$tmp" "$err"' EXIT

if ! "$BIN" "$id" -f markdown -o "$tmp" 2>"$err"; then
  cat "$err" >&2
  exit 1
fi

[ -s "$tmp" ] || { echo "error: transcript came back empty" >&2; exit 1; }
mv "$tmp" "$out"
echo "$out"
