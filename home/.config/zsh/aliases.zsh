# ============================================================================
#  dotfiles/zsh/aliases.zsh
# ============================================================================
#
#  Modern tools REPLACE the classic ones below. Escape hatches when you need
#  the original POSIX behaviour (e.g. in a one-off command):
#     command grep ...     OR     \grep ...
#  Aliases are interactive-only and are NOT expanded inside scripts, so your
#  scripts keep using the real /usr/bin tools.
# ----------------------------------------------------------------------------

# Claude Code's shell (CLAUDECODE=1) gets the real tools: its commands broke on
# ls=eza, cat=bat, ps=procs, du=dust and mkdir -p in 56 sessions.
[[ -n $CLAUDECODE ]] && return

# ---- ls -> eza -------------------------------------------------------------
if command -v eza >/dev/null; then
  alias ls='eza --icons --group-directories-first'
  alias l='eza -1 --icons --group-directories-first'
  alias ll='eza -lh --icons --group-directories-first --git'
  alias la='eza -lah --icons --group-directories-first --git'
  alias lt='eza --tree --level=2 --icons --group-directories-first'
  alias ltt='eza --tree --level=3 --icons --group-directories-first'
fi

# ---- cat -> bat ------------------------------------------------------------
if command -v bat >/dev/null; then
  alias cat='bat --paging=never'
  alias catp='bat'                 # paged view
fi

# ---- grep/find/du/df/ps/top -> modern equivalents -------------------------
command -v rg    >/dev/null && alias grep='rg'
command -v fd    >/dev/null && alias find='fd'
command -v dust  >/dev/null && alias du='dust'
command -v duf   >/dev/null && alias df='duf'
command -v procs >/dev/null && alias ps='procs'
command -v btop  >/dev/null && alias top='btop'
# (sd is great for simple find/replace but its syntax differs from sed, so
#  it's left un-aliased — call it as `sd 'find' 'replace' file`.)

# ---- handy tool shortcuts --------------------------------------------------
command -v lazygit    >/dev/null && alias lg='lazygit'
command -v lazydocker >/dev/null && alias lzd='lazydocker'
command -v gh         >/dev/null && alias ghd='gh dash'    # gh-dash extension
command -v just       >/dev/null && alias j='just'
command -v yazi       >/dev/null && alias ya='yazi'       # `y` (function) cd's on quit
command -v glow       >/dev/null && alias md='glow'        # render Markdown
command -v xh         >/dev/null && alias http='xh'
command -v btop       >/dev/null && alias btm='btop'
command -v tldr       >/dev/null && alias help='tldr'
command -v fastfetch  >/dev/null && alias ff='fastfetch'

# ---- git -------------------------------------------------------------------
alias g='git'
alias gs='git status -sb'
alias ga='git add'
alias gaa='git add -A'
alias gc='git commit'
alias gcm='git commit -m'
alias gca='git commit --amend'
alias gco='git checkout'
alias gsw='git switch'
alias gb='git branch'
alias gd='git diff'
alias gds='git diff --staged'
alias gl='git log --oneline --graph --decorate -20'
alias gll='git log --graph --pretty=format:"%C(auto)%h%d %s %C(blue)(%cr) %C(green)<%an>"'
alias gp='git push'
alias gpl='git pull'
alias gf='git fetch --all --prune'
alias grb='git rebase'
alias gst='git stash'

# ---- pnpm / node -----------------------------------------------------------
command -v pnpm >/dev/null && {
  alias pn='pnpm'
  alias pni='pnpm install'
  alias pnd='pnpm dev'
  alias pnb='pnpm build'
  alias pnx='pnpm dlx'
}

# ---- navigation & misc -----------------------------------------------------
alias ..='cd ..'
alias ...='cd ../..'
alias ....='cd ../../..'
alias mkdir='mkdir -p'
alias reload='exec zsh'
alias dot='cd "$DOTFILES"'
alias path='echo -e ${PATH//:/\\n}'
if [[ $OSTYPE == darwin* ]]; then
  alias ports='lsof -iTCP -sTCP:LISTEN -n -P'
  alias localip='ipconfig getifaddr en0'
else
  alias ports='ss -tlnp'
  alias localip="hostname -I | awk '{print \$1}'"
fi
alias myip='curl -s ifconfig.me; echo'

# ---- dotfiles (workflow: edit source -> it's live; `dots` for new files) --
if [[ -x "$DOTFILES/dotfiles" ]]; then
  alias dots="$DOTFILES/dotfiles"       # pull + link + provision (install & sync, one command)
  alias dotd='git -C "$DOTFILES" diff'  # preview source changes  (`dot` already cd's into $DOTFILES)
  alias cursor-rules='"$DOTFILES/scripts/agent-config" user-rules --copy'  # rules for every Cursor folder
  alias zshrc="\${EDITOR:-nano} \"\$DOTFILES/home/.config/zsh/init.zsh\""
fi
