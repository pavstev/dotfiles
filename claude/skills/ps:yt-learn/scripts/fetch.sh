#!/usr/bin/env bash
# fetch.sh <youtube url or id> — print the path of a timestamped markdown transcript.
# Cache: ~/.cache/yt-learn/<video id>.md (a rerun is free).
# Tool: yt-transcribe from powertools. Set POWERTOOLS to move the checkout.
set -euo pipefail

POWERTOOLS=${POWERTOOLS:-$HOME/powertools}

url="${1:-}"
[ -n "$url" ] || { echo "usage: fetch.sh <youtube url>" >&2; exit 2; }

id="$(printf '%s' "$url" | sed -nE 's#.*(youtu\.be/|[?&]v=|/shorts/|/embed/|/live/)([A-Za-z0-9_-]{11}).*#\2#p' | head -n1)"
[ -n "$id" ] || id="$(printf '%s' "$url" | grep -E '^[A-Za-z0-9_-]{11}$' || true)"
[ -n "$id" ] || { echo "error: no YouTube video id found in: $url" >&2; exit 1; }

[ -d "$POWERTOOLS/yt-transcribe" ] || {
  echo "error: yt-transcribe not found at $POWERTOOLS/yt-transcribe. Set POWERTOOLS to the powertools folder." >&2
  exit 1
}

dir="$HOME/.cache/yt-learn"
out="$dir/$id.md"
mkdir -p "$dir"

if [ -s "$out" ]; then
  echo "$out"
  exit 0
fi

tmp="$(mktemp "$dir/.$id.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

err="$(mktemp)"
trap 'rm -f "$tmp" "$err"' EXIT

if ! uv run --project "$POWERTOOLS/yt-transcribe" yt-transcribe "$id" -f markdown -o "$tmp" 2>"$err"; then
  cat "$err" >&2
  if grep -qi 'ffmpeg' "$err" || ! command -v ffmpeg >/dev/null 2>&1; then
    echo "hint: the Whisper fallback needs ffmpeg. Run: brew install ffmpeg" >&2
  fi
  exit 1
fi

[ -s "$tmp" ] || { echo "error: transcript came back empty" >&2; exit 1; }
mv "$tmp" "$out"
echo "$out"
