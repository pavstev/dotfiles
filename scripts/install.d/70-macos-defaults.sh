#!/usr/bin/env bash
# Curated macOS defaults for development.
# Best-effort (no `set -e`): if one key is unsupported on a given macOS version,
# the remaining settings still apply.
set -uo pipefail
. "$REPO/scripts/setup-lib.sh"
only_macos "macOS defaults"

step "macOS defaults"
info "Applying curated development defaults…"

# --- Keyboard: fast key repeat, disable press-and-hold accent popup ---------
defaults write -g KeyRepeat -int 2
defaults write -g InitialKeyRepeat -int 15
defaults write -g ApplePressAndHoldEnabled -bool false

# --- Finder: extensions, path/status bars, list view, sane search -----------
defaults write -g AppleShowAllExtensions -bool true
defaults write com.apple.finder AppleShowAllFiles -bool true
defaults write com.apple.finder ShowPathbar -bool true
defaults write com.apple.finder ShowStatusBar -bool true
defaults write com.apple.finder _FXShowPosixPathInTitle -bool true
defaults write com.apple.finder FXPreferredViewStyle -string "Nlsv"
defaults write com.apple.finder FXDefaultSearchScope -string "SCcf"   # search current folder
defaults write com.apple.finder FXEnableExtensionChangeWarning -bool false
defaults write com.apple.desktopservices DSDontWriteNetworkStores -bool true
defaults write com.apple.desktopservices DSDontWriteUSBStores -bool true

# --- Save/print panels expanded by default ----------------------------------
defaults write -g NSNavPanelExpandedStateForSaveMode -bool true
defaults write -g NSNavPanelExpandedStateForSaveMode2 -bool true
defaults write -g PMPrintingExpandedStateForPrint -bool true

# --- Screenshots: PNG into ~/Screenshots, no window shadow ------------------
mkdir -p "$HOME/Screenshots" 2>/dev/null || true
defaults write com.apple.screencapture location -string "$HOME/Screenshots"
defaults write com.apple.screencapture type -string "png"
defaults write com.apple.screencapture disable-shadow -bool true

# --- Dock: snappier autohide, don't auto-rearrange Spaces -------------------
defaults write com.apple.dock autohide-time-modifier -float 0.15
defaults write com.apple.dock mru-spaces -bool false

# --- Misc: no auto-capitalisation or auto-correct in code -------------------
defaults write -g NSAutomaticCapitalizationEnabled -bool false
defaults write -g NSAutomaticSpellingCorrectionEnabled -bool false

for app in Finder Dock SystemUIServer; do killall "$app" >/dev/null 2>&1 || true; done
ok "macOS defaults applied (a few settings need logout/login to fully take effect)"
