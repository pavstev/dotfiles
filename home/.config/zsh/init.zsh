# ============================================================================
#  ~/.config/zsh/init.zsh  —  main shell config (symlinked from this repo).
#  Source: $DOTFILES/home/.config/zsh/init.zsh. Sourced by ~/.zshrc.
#  Edits here are live immediately — no apply step.
# ============================================================================
DOTFILES="${DOTFILES:-$HOME/dotfiles}"

# ---- Homebrew (Apple Silicon or Intel) ------------------------------------
if [[ -x /opt/homebrew/bin/brew ]]; then
  eval "$(/opt/homebrew/bin/brew shellenv)"
elif [[ -x /usr/local/bin/brew ]]; then
  eval "$(/usr/local/bin/brew shellenv)"
fi

# ---- Oh My Zsh -------------------------------------------------------------
export ZSH="$HOME/.oh-my-zsh"
ZSH_THEME=""                       # prompt is handled by Starship
DISABLE_AUTO_TITLE="false"
ENABLE_CORRECTION="false"
COMPLETION_WAITING_DOTS="true"
zstyle ':omz:update' mode disabled  # don't let OMZ self-update interactively

# Order matters: fzf-tab BEFORE highlighting; highlighting/autosuggestions last.
plugins=(
  git
  fzf-tab
  zsh-autosuggestions
  fast-syntax-highlighting
  zsh-completions
)

# zsh-completions needs its functions on fpath before compinit (OMZ runs it).
fpath+=("$ZSH/custom/plugins/zsh-completions/src")

# compaudit only trusts root/current-user ownership; Ghostty/OrbStack completion
# files under /Applications are owned by this Mac's other account, a benign
# false positive on a single-human machine — skip the check.
ZSH_DISABLE_COMPFIX="true"
source "$ZSH/oh-my-zsh.sh"

# ---- History ---------------------------------------------------------------
HISTFILE="$HOME/.zsh_history"
HISTSIZE=100000
SAVEHIST=100000
setopt EXTENDED_HISTORY INC_APPEND_HISTORY SHARE_HISTORY
setopt HIST_IGNORE_DUPS HIST_IGNORE_ALL_DUPS HIST_IGNORE_SPACE
setopt HIST_REDUCE_BLANKS HIST_VERIFY HIST_FIND_NO_DUPS

# ---- Shell behaviour -------------------------------------------------------
setopt INTERACTIVE_COMMENTS   # let `#` start a comment on the command line

# ---- Completion styling ----------------------------------------------------
zstyle ':completion:*' menu no                       # let fzf-tab own the menu
zstyle ':completion:*' matcher-list 'm:{a-zA-Z}={A-Za-z}' 'r:|=*' 'l:|=* r:|=*'
zstyle ':completion:*' list-colors "${(s.:.)LS_COLORS}"
zstyle ':completion:*:descriptions' format '[%d]'

# fzf-tab: rich previews
zstyle ':fzf-tab:complete:cd:*'         fzf-preview 'eza -1 --color=always --icons $realpath'
zstyle ':fzf-tab:complete:__zoxide_z:*' fzf-preview 'eza -1 --color=always --icons $realpath'
zstyle ':fzf-tab:complete:*:*'          fzf-preview 'bat --color=always --style=numbers --line-range=:200 $realpath 2>/dev/null || eza -1 --color=always --icons $realpath 2>/dev/null'
zstyle ':fzf-tab:*' switch-group '<' '>'
zstyle ':fzf-tab:*' fzf-flags --height=60% --layout=reverse --border

# ---- fzf (Ctrl-T files, Alt-C cd; Ctrl-R is handled by atuin below) -------
if command -v fzf >/dev/null; then
  source <(fzf --zsh) 2>/dev/null || true
  export FZF_DEFAULT_COMMAND='fd --type f --hidden --follow --exclude .git'
  export FZF_DEFAULT_OPTS="
    --height 60% --layout=reverse --border --info=inline
    --prompt='❯ ' --pointer='▶' --marker='✓'
    --color=fg:#EDE8E3,bg:#1C1917,hl:#E0825F,fg+:#EDE8E3,bg+:#292524,hl+:#E8916A
    --color=info:#A8A29E,prompt:#E0825F,pointer:#E8916A,marker:#87B083,spinner:#E0825F,header:#A8A29E"
  export FZF_CTRL_T_COMMAND="$FZF_DEFAULT_COMMAND"
  export FZF_CTRL_T_OPTS="--preview 'bat --color=always --style=numbers --line-range=:300 {} 2>/dev/null || eza -1 --color=always --icons {}'"
  export FZF_ALT_C_COMMAND='fd --type d --hidden --follow --exclude .git'
  export FZF_ALT_C_OPTS="--preview 'eza -1 --color=always --icons {}'"
fi

# ---- atuin (synced history on Ctrl-R; ↑ stays native) --------------------
# Load AFTER fzf so atuin wins the Ctrl-R binding. fzf keeps Ctrl-T / Alt-C.
command -v atuin >/dev/null && eval "$(atuin init zsh --disable-up-arrow)"

# ---- Tooling hooks ---------------------------------------------------------
command -v zoxide   >/dev/null && eval "$(zoxide init zsh)"            # smart cd  (z <dir>)
command -v direnv   >/dev/null && eval "$(direnv hook zsh)"           # per-dir env
command -v mise     >/dev/null && eval "$(mise activate zsh)"         # runtime versions
command -v carapace >/dev/null && source <(carapace _carapace zsh)    # multi-shell completions
command -v starship >/dev/null && eval "$(starship init zsh)"         # prompt (load last)
# starship reads ~/.config/starship.toml automatically (symlinked from this repo).

# ---- zsh-autosuggestions tuning -------------------------------------------
ZSH_AUTOSUGGEST_STRATEGY=(history completion)
ZSH_AUTOSUGGEST_HIGHLIGHT_STYLE="fg=8"
bindkey '^ ' autosuggest-accept       # Ctrl-Space to accept a suggestion

# ---- Environment -----------------------------------------------------------
export EDITOR="nano"
export VISUAL="$EDITOR"
export LANG="en_US.UTF-8"
export CLICOLOR=1
export PAGER="less"
export LESS="-R"
export BAT_THEME="ansi"   # inherits the terminal's own colour palette
if command -v bat >/dev/null; then
  export MANPAGER="sh -c 'col -bx | bat -l man -p'"   # colorized man pages
  export MANROFFOPT="-c"
fi
export TEALDEER_CONFIG_DIR="$HOME/.config/tealdeer"   # macOS default is ~/Library/...; relocate to XDG

# ---- Keybindings -----------------------------------------------------------
bindkey '^[[1;5C' forward-word                # Ctrl-Right
bindkey '^[[1;5D' backward-word               # Ctrl-Left
bindkey '^A' beginning-of-line
bindkey '^E' end-of-line

# ---- PATH (Rust + uv tools) -----------------------------------------------
# Rust: Homebrew's rustup exposes cargo/rustc proxies under
# $HOMEBREW_PREFIX/opt/rustup/bin; a rustup-init install uses ~/.cargo/bin.
[[ -n "$HOMEBREW_PREFIX" && -d "$HOMEBREW_PREFIX/opt/rustup/bin" ]] && path=("$HOMEBREW_PREFIX/opt/rustup/bin" $path)
[[ -d "$HOME/.cargo/bin" ]] && path=("$HOME/.cargo/bin" $path)
[[ -d "$HOME/.local/bin" ]] && path=("$HOME/.local/bin" $path)   # `uv tool` installs

# ---- yazi (file manager; `y` cd's to the last dir on quit) ----------------
if command -v yazi >/dev/null; then
  y() {
    local tmp cwd
    tmp="$(mktemp -t yazi-cwd.XXXXXX)"
    yazi "$@" --cwd-file="$tmp"
    if cwd="$(command cat -- "$tmp" 2>/dev/null)" && [[ -n "$cwd" && "$cwd" != "$PWD" ]]; then
      builtin cd -- "$cwd"
    fi
    rm -f -- "$tmp"
  }
fi

# ---- Aliases (modular) -----------------------------------------------------
[[ -f "$HOME/.config/zsh/aliases.zsh" ]] && source "$HOME/.config/zsh/aliases.zsh"

# ---- skill CLI (install/list/remove/create/status) -------------------------
[[ -f "$HOME/.config/zsh/skill.zsh" ]] && source "$HOME/.config/zsh/skill.zsh"
