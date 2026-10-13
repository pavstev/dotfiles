# ============================================================================
#  ~/.zshrc  —  symlinked from this repo.
#  Edit the real config at  $DOTFILES/home/.config/zsh/*.zsh  — it's live
#  immediately (no apply step). Run `dots` only to pick up new files/packages.
# ============================================================================
export DOTFILES="${DOTFILES:-$HOME/dotfiles}"

# Main shell config.
[[ -f "$HOME/.config/zsh/init.zsh" ]] && source "$HOME/.config/zsh/init.zsh"

# Machine-local overrides (gitignored; not managed by this repo).
[[ -f "$HOME/.config/zsh/local.zsh" ]] && source "$HOME/.config/zsh/local.zsh"
[[ -f "$HOME/.zshrc.local" ]] && source "$HOME/.zshrc.local"
