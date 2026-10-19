#!/usr/bin/env bash
# fetch.sh <youtube url or id>: print the path of a timestamped markdown transcript.
# Cache: ~/.cache/yt-learn/<video id>.md (a rerun is free).
# Tool: the yt_transcribe Rust bin in this repo (rust/agent-config), run through claude/hooks/rust-bin.
# `dots` builds it; this script never does. It needs yt-dlp; the Whisper fallback also needs
# ffmpeg and whisper-cli (it says how to install each one when it is missing).
set -euo pipefail

url="${1:-}"
[ -n "$url" ] || { echo "usage: fetch.sh <youtube url>" >&2; exit 2; }

# The skill folder is a symlink into the repo; resolve it to find the launcher.
HERE="$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
RB="$HERE/../../../hooks/rust-bin"
[ -f "$RB" ] || RB="${DOTFILES_REPO:-$HOME/dotfiles}/claude/hooks/rust-bin"
[ -f "$RB" ] || { echo "error: dotfiles repo not found. Set DOTFILES_REPO." >&2; exit 1; }
BIN=(bash "$RB" yt_transcribe)

# One parser for every link form: --print-id (pure, no network).
id="$("${BIN[@]}" --print-id "$url")" || exit 1

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

if ! "${BIN[@]}" "$id" -f markdown -o "$tmp" 2>"$err"; then
  cat "$err" >&2
  exit 1
fi

[ -s "$tmp" ] || { echo "error: transcript came back empty" >&2; exit 1; }
mv "$tmp" "$out"
echo "$out"
