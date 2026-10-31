#!/usr/bin/env bash
# Sets the default app per file type from default-apps.list, with no Finder pop-up.
# macOS only: on any other OS it does nothing and exits 0. Safe to run again.
# Prints "changed N" or "ok N" (see default-apps.swift). Run alone: scripts/default-apps.sh
set -euo pipefail

[ "$(uname -s)" = "Darwin" ] || exit 0

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
list="${1:-$here/../default-apps.list}"

[ -f "$list" ] || { echo "default-apps: $list not found" >&2; exit 0; }
command -v swift >/dev/null 2>&1 || { echo "default-apps: swift not found (run: xcode-select --install)" >&2; exit 0; }

exec swift "$here/default-apps.swift" "$list"
