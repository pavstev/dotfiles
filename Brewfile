# Brewfile — `brew bundle --file=Brewfile` installs only what's missing.
# Applied by scripts/install.d/20-brew-bundle.sh, run every time `./dotfiles` runs.
# Per-machine extras go in Brewfile.local (gitignored) — see Brewfile.local.example.
# On Linux the same step drops the tap, cask, mas and vscode lines (Mac only) and installs the rest
# with Homebrew on Linux. Keep each of those lines on one line starting with its keyword.

# ---- Taps ------------------------------------------------------------------
tap "nikitabobko/tap"     # AeroSpace tiling window manager
# (gh-dash is a gh extension, installed by scripts/install.d/55-gh-extensions.sh)

# ---- Dotfiles engine -------------------------------------------------------
brew "age"             # modern file encryption (secrets, if you ever need them)

# ---- Shell experience ------------------------------------------------------
brew "starship"        # prompt
brew "zsh-completions" # extra completions
brew "carapace"        # multi-shell completion engine (1000+ commands)
brew "atuin"           # synced, searchable shell history (Ctrl-R)

# ---- Core navigation / search ---------------------------------------------
brew "fzf"             # fuzzy finder (Ctrl-T files / Alt-C cd)
brew "zoxide"          # smart cd
brew "ripgrep"         # rg  -> grep
brew "fd"              # fd  -> find
brew "ast-grep"        # sg  -> structural (AST) search & rewrite

# ---- Modern file / system tools -------------------------------------------
brew "eza"             # eza  -> ls
brew "bat"             # bat  -> cat
brew "yazi"            # blazing terminal file manager (y -> cd on quit)
brew "dust"            # dust -> du
brew "duf"             # duf  -> df
brew "procs"           # procs-> ps
brew "btop"            # btop -> top
brew "sd"              # simple find/replace (sed-ish)
brew "glow"            # render Markdown in the terminal

# ---- Git -------------------------------------------------------------------
brew "git-delta"               # gorgeous diffs
brew "difftastic"              # structural (syntax-aware) diff
brew "lazygit"                 # git TUI
brew "gh"                      # GitHub CLI  (+ gh-dash extension → `gh dash`)

# ---- Dev workflow ----------------------------------------------------------
brew "direnv"          # per-directory env
brew "mise"            # node/python/etc version manager
brew "just"            # command runner (modern make)
brew "jq"              # JSON processor
brew "yq"              # YAML/JSON/XML processor
brew "xh"              # fast HTTPie-style HTTP client
brew "gum"             # pretty interactive shell-script prompts (Charm)
brew "tealdeer"        # tldr  -> fast cheat sheets
brew "hyperfine"       # command-line benchmarking
brew "fastfetch"       # fancy system info
brew "opencode"        # AI coding agent in the terminal (reads ~/.claude/skills)

# ---- Containers ------------------------------------------------------------
brew "lazydocker"      # docker/compose TUI

# ---- Python (Astral, Rust-powered) ----------------------------------------
brew "uv"              # packaging/venv/python-version manager (pip/poetry/pyenv/pipx)
brew "ruff"            # linter + formatter
brew "ty"              # type checker (preview)

# ---- JavaScript / TypeScript ----------------------------------------------
brew "biome"           # formatter + linter (Prettier + ESLint, in Rust)

# ---- Go --------------------------------------------------------------------
brew "golangci-lint"   # meta-linter
brew "air"             # live reload for Go apps

# ---- Rust ------------------------------------------------------------------
brew "rustup"          # toolchain manager (cargo/rustc); run `rustup default stable`
brew "cargo-binstall"  # install cargo tools from prebuilt binaries
brew "cargo-nextest"   # next-gen test runner
brew "bacon"           # background checker (clippy/check/test TUI)

# ---- macOS apps & fonts ----------------------------------------------------
cask "ghostty"
cask "visual-studio-code"
cask "cursor"                        # AI-first editor (mirrors VS Code config). Agent CLI comes from the
                                     # official installer in scripts/install.d/60-cursor.sh, not cask "cursor-cli".
cask "raycast"                       # launcher (settings sync through its own account)
cask "orbstack"                      # fast Docker Desktop / Linux VM alternative
cask "nikitabobko/tap/aerospace"     # tiling window manager
cask "font-fira-code-nerd-font"

# ---- VS Code / Cursor extensions -------------------------------------------
# `brew bundle` installs these idempotently via the `code` CLI (VS Code only).
# scripts/install.d/60-cursor.sh installs the same list into Cursor with
# `cursor --install-extension`. It skips ms-python.vscode-pylance (not on Open VSX).
vscode "zhuangtongfa.material-theme"    # One Dark Pro theme
vscode "biomejs.biome"                  # Biome formatter + linter
vscode "rust-lang.rust-analyzer"        # Rust language server
vscode "golang.go"                      # Go language server
vscode "ms-python.python"               # Python language support
vscode "ms-python.vscode-pylance"       # Python type checking (Pylance)
vscode "charliermarsh.ruff"             # Ruff linter/formatter for Python
vscode "tamasfe.even-better-toml"       # TOML (Cargo.toml, pyproject.toml, etc.)
vscode "redhat.vscode-yaml"             # YAML language server
vscode "github.vscode-github-actions"   # GitHub Actions workflow support

# ---- Mac App Store ---------------------------------------------------------
brew "mas"             # install App Store apps from a Brewfile (mas "App", id: ...)
