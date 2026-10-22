# Platforms

What is different on Linux and on Windows, and how to test each one.

← [Docs](README.md)

The code asks the OS with `is_macos`, `is_linux` and `is_windows` (`scripts/setup-lib.sh`). Rules
for changing it: [AGENTS.md](../AGENTS.md#platforms).

## Linux

`./dotfiles` runs on Debian, Ubuntu, Fedora, Arch and Alpine (x86_64 and arm64) as a normal user
with `sudo`. A distro without apt, dnf, pacman or apk gets the commands to run yourself; the run
goes on.

- **Packages:** step `05-linux-prereqs` installs build tools, `curl`, `file`, `git`, `procps` and
  `zsh` (names per manager: `scripts/linux-prereqs.txt`). Step `12-tools` installs the tools of
  `tools.list` with mise (no Homebrew on Linux): mise first, then each tool from the first source
  that works. Same tools and versions as macOS, no `batcat` or `fdfind`.
- **Without `sudo`:** install the packages of `scripts/linux-prereqs.txt` as root. The tools need
  no root: mise installs them under your home.
- **Alpine (musl):** eza, procs, delta, difftastic, fastfetch, ffmpeg and poppler come from `apk`
  first (no musl release), the rest from mise. That needs root or `sudo`. whisper-cpp is not
  installed. A tool with no source that starts on musl gives a warning.
- **Skipped:** the macOS-only steps (`10-homebrew`, `20-brew-bundle`, `70-macos-defaults`,
  `72-touch-id-sudo`, `75-terminal-appearance`) print nothing (`dots -v` shows
  `macOS only, skipping`). The Claude desktop app offer, AeroSpace and the Nerd
  Font are macOS only too.
- **Links:** `~/.config/Code/User/*` and `~/.config/Cursor/User/*` link to the files macOS keeps
  under `~/Library/Application Support`. Ghostty reads `~/.config/ghostty` on both.
- **Shell:** `init.zsh` puts `~/.local/bin` and the mise shims on PATH. It falls back to `C.UTF-8`
  when `en_US.UTF-8` is not built. Without VS Code it sets `GIT_EDITOR` to `$EDITOR` in
  interactive zsh. Git from bash or a hook still uses `code --wait` from `~/.gitconfig`.
- **Login shell:** not changed. `78-linux-extras` prints the `chsh` line when it is not zsh.
- **Skills:** `ps:bag-mode` is for a Mac laptop; on Linux and Windows it says so and stops. The
  other skill scripts run on all three.

### Test it

`scripts/test-linux.sh <image>` runs `./dotfiles` twice in a fresh container, as a user with
`sudo`. It needs podman (rootless is enough) or docker. A run takes 10 to 25 minutes.

```sh
bash scripts/test-linux.sh debian:13
bash scripts/test-linux.sh ubuntu:24.04
```

Run 1 must exit 0 with no warning. Run 2 must change nothing. It has no terminal, so the menus,
password prompts and sign-in step are not covered. What each test checks:
[AGENTS.md](../AGENTS.md#tests).

To test on another Linux machine, set `TEST_HOST` (an SSH host name) in `dots settings` and run
`scripts/os-check.sh linux`. When that host is off, `scripts/os-check.sh linux --local` uses
local podman or docker instead. With no OS, `scripts/os-check.sh` checks every OS whose files
changed. It is manual only. In Claude Code, run `/os-check`.

The Windows check needs the VM `win11-golden` once:

1. Install UTM: `brew install --cask utm`.
2. Save the Windows 11 Arm64 ISO from microsoft.com/software-download/windows11arm64 as
   `~/VMs/iso/Win11_arm64_en-us.iso`.
3. Run `bash scripts/win-golden.sh`. It installs Windows unattended (about 30 minutes).

Each check then works on a clone of the golden VM and deletes the clone.

## Windows

`./dotfiles` runs in Git Bash (MSYS/MINGW). WSL counts as Linux and follows the Linux path.

- **Packages:** no Homebrew. Two steps run: `12-tools` and `57-claude-code`. `12-tools` installs
  mise with WinGet (`jdx.mise`), then the tools of `tools.list` marked for Windows, from mise
  first. WinGet and Scoop are fallback sources (Opencode `SST.opencode`, yt-dlp, ffmpeg, rustup).
  `57-claude-code` uses `Anthropic.ClaudeCode`, then the PowerShell installer. The shell-only
  tools (starship, atuin, zoxide, eza, yazi, bottom and others) are for macOS and Linux only,
  because zsh does not run on Windows. mise and its shims go on PATH for the run; open a new Git
  Bash afterwards.
- **Links:** real symlinks, so Developer Mode must be on (the script stops and says how). These
  link: `.config/gh/config.yml`, the Claude config under `~/.claude`, and the VS Code and Cursor
  settings (`%APPDATA%\{Code,Cursor}\User\`, only with the `editor` look). `~/.gitconfig` is not
  linked: the git identity goes to `~/.gitconfig.local`, which your own `~/.gitconfig` includes.
- **Skipped:** the same macOS-only steps and apps as on Linux.
- **Shell:** Git Bash provides `git`, `ln -s` and `clip`. There is no zsh, so the zsh config is
  not linked. `scripts/agent-config user-rules --copy` (the Cursor User Rules text) uses `clip`.
- **Cursor and Opencode config:** the same output as on macOS and Linux.
- **Rust:** with no Rust, `dots` installs it first (the `x86_64-pc-windows-gnu` host, no Visual
  Studio). Python is not needed.
- **Remove it:** `dots uninstall` ([Customize](customize.md#uninstall)). It removes the links and
  keeps the repo.
