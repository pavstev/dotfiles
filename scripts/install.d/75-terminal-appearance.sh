#!/usr/bin/env bash
# Apple Terminal appearance: Nerd Font glyphs + a smaller, compact profile.
# Ghostty is configured via its own dotfile; Terminal.app stores font and window
# geometry in the app's profile, so we drive it through AppleScript here.
#
# Uses the authoritative PostScript name "FiraCodeNFM-Reg" (FiraCode Nerd Font
# Mono, Regular). Requires the font-fira-code-nerd-font cask (installed by brew).
# Tolerant: if Automation permission isn't granted yet it warns and moves on.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
only_macos "Apple Terminal appearance"

FONT_PS="FiraCodeNFM-Reg"   # FiraCode Nerd Font Mono — Regular
FONT_SIZE=12                # compact
COLUMNS_=100                # tidy, compact default window…
ROWS_=28                    # …geometry (Terminal has no padding control)

step "Apple Terminal appearance"

have osascript || { warn "osascript unavailable — skipping Terminal."; exit 0; }

# Confirm the font is installed (in EITHER location) before touching Terminal.
if [[ ! -f "$HOME/Library/Fonts/FiraCodeNerdFontMono-Regular.ttf" \
   && ! -f "/Library/Fonts/FiraCodeNerdFontMono-Regular.ttf" ]]; then
  warn "FiraCode Nerd Font not installed yet — skipping Terminal appearance."
  exit 0
fi

if osascript >/dev/null 2>&1 <<OSA
tell application "Terminal"
    -- "Basic" may have been renamed; wrap it so a miss doesn't block the rest.
    try
        set font name of settings set "Basic" to "$FONT_PS"
        set font size of settings set "Basic" to $FONT_SIZE
        set number of columns of settings set "Basic" to $COLUMNS_
        set number of rows of settings set "Basic" to $ROWS_
    end try
    -- The default + startup profiles always resolve, whatever they're named.
    set font name of default settings to "$FONT_PS"
    set font size of default settings to $FONT_SIZE
    set number of columns of default settings to $COLUMNS_
    set number of rows of default settings to $ROWS_
    set font name of startup settings to "$FONT_PS"
    set font size of startup settings to $FONT_SIZE
end tell
OSA
then
  ok "Apple Terminal → FiraCode Nerd Font Mono ${FONT_SIZE}pt, ${COLUMNS_}×${ROWS_} (compact, icons on)"
else
  warn "Couldn't set Apple Terminal appearance automatically."
  warn "Grant Automation permission (System Settings → Privacy → Automation), then re-run: ./dotfiles"
fi
