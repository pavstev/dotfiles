# Brewfile: the macOS apps. `brew bundle --file=Brewfile` installs only what is missing.
# Applied by step 20-brew-bundle (macOS only), run every time `./dotfiles` runs.
# Per-machine extras go in Brewfile.local (gitignored), see Brewfile.local.example.
#
# Command-line tools are NOT here. tools.list lists them, and scripts/tools installs them with
# mise on macOS, Linux and Windows. A tool with no other source on macOS may get a `brew` line here.
# Keep each line on one line starting with its keyword.

# ---- Taps ------------------------------------------------------------------
tap "nikitabobko/tap"     # AeroSpace tiling window manager
# (gh-dash is a gh extension, installed by step 55-gh-extensions)

# ---- macOS apps & fonts ----------------------------------------------------
cask "ghostty"
cask "visual-studio-code"
cask "cursor"                        # AI-first editor (mirrors VS Code config). Agent CLI comes from the
                                     # official installer in step 60-cursor, not cask "cursor-cli".
cask "raycast"                       # launcher (settings sync through its own account)
cask "orbstack"                      # fast Docker Desktop / Linux VM alternative
cask "nikitabobko/tap/aerospace"     # tiling window manager
cask "font-fira-code-nerd-font"

# ---- VS Code / Cursor extensions -------------------------------------------
# `brew bundle` installs these idempotently via the `code` CLI (VS Code only).
# step 60-cursor installs the same list into Cursor with
# `cursor --install-extension`. It skips ms-python.vscode-pylance (not on Open VSX).
# step 78-linux-extras installs them into VS Code on Linux.
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
