# Platforms

What is different on Linux and on Windows, and how to test each one.

← [Docs](README.md)

## Linux

`./dotfiles` runs on Debian, Ubuntu, Fedora and Arch (x86_64 and arm64) as a normal user with `sudo`.
The code asks the OS with `is_macos`, `is_linux` and `is_windows` (`scripts/setup-lib.sh`). On Linux:

- **Packages:** step `05-linux-prereqs` installs build tools, `curl`, `file`, `git`, `procps`
  and `zsh` with apt, dnf, pacman or apk (`scripts/linux-prereqs.txt`
  has the names for each). Step `12-tools` then installs the command-line tools of `tools.list`
  with mise (no Homebrew on Linux): it installs mise first, then each tool from the first source
  that works. Same tools and versions as macOS, and no `batcat` or `fdfind`. On Alpine the native
  `apk` package comes first for the tools whose release binaries need glibc.
- **Skipped on Linux:** `70-macos-defaults`, `72-touch-id-sudo` and `75-terminal-appearance`
  print nothing (`dots -v` shows `macOS only, skipping`). The Claude desktop app offer, AeroSpace and the Fira Code Nerd Font
  are macOS-only too.
- **Links:** `~/.config/Code/User/*` and `~/.config/Cursor/User/*` link to the same
  files that macOS gets under `~/Library/Application Support`. Ghostty reads
  `~/.config/ghostty` on both.
- **Shell:** `init.zsh` puts `~/.local/bin` and the mise shims on PATH, falls back to `C.UTF-8` when `en_US.UTF-8` is
  not built, and sets `GIT_EDITOR` to `$EDITOR` in interactive zsh when VS Code is not installed
  (`~/.gitconfig` asks for `code --wait`; git from bash or a hook still uses that).
- **Not changed:** the login shell. `78-linux-extras` prints the `chsh` line when it is not zsh.
- **Without `sudo`:** install the packages in `scripts/linux-prereqs.txt` as root.
  The tools need no root: mise installs them under your home. Only the `apk` source (Alpine) and the
  prerequisites use root.
- **Alpine (musl):** the tools come from `apk` first for the ones that have no musl release (eza, procs,
  delta, difftastic, fastfetch, ffmpeg, poppler), else from mise. whisper-cpp is not installed on Alpine. A tool with no source that
  starts on musl is a warning. Needs root or `sudo` for `apk`.
- **Other distros:** without apt, dnf, pacman or apk, the prerequisites step prints the commands
  to install yourself and carries on.
- **Skills:** `ps:bag-mode` is for a Mac laptop. On Linux and Windows it says so and stops.
  The other skill scripts run on all three.

### Test it

`scripts/test-linux.sh <image>` runs `./dotfiles` twice in a fresh container, as a user with
`sudo`, on a host with podman or docker (rootless podman is enough):

```sh
bash scripts/test-linux.sh debian:13
bash scripts/test-linux.sh ubuntu:24.04
```

It builds a small base image once, copies the working tree in (git-ignored files stay out),
runs `./dotfiles` and again `./dotfiles`, and checks: run 1 exits 0 with no warning and no shell
error, both runs end with the done line, run 2 changes nothing and takes 90 seconds at most,
`dots -v` works, the links, the tools, the Rust tools, `zsh -ic exit` prints nothing, `bash -n`
on every shell script, and that the Rust skill tools run (`skill_check`, `mic_check`,
`whatsleft_scan`, `hurry_scan`, `harness_stats`, `bag_check`, `statusline`). The Rust unit tests run
on the host with `cargo test`, not in the container. `--keep` leaves the container for a look
(`podman exec -it dots-test-debian-13 bash`), `--reuse` syncs your edits into it and runs again.
It runs without a terminal, so the gum menus, the password prompts and the sign-in step are
not covered. A run takes 10 to 25 minutes; most of it is the tool downloads and the Rust build.

To run it on another Linux machine, set `TEST_HOST` (an SSH host name) in `dots settings` and run
`scripts/os-check.sh linux`. It copies the tree there and runs `test-linux.sh` for each image in
`OS_CHECK_IMAGES`. `scripts/os-check.sh` with no OS checks every OS whose files changed: macOS,
Linux and Windows (a throwaway UTM clone of `win11-golden`). It is manual only. In Claude Code, run
`/os-check`.

The Windows check needs the VM `win11-golden` once: install UTM (`brew install --cask utm`), save
the Windows 11 Arm64 ISO from microsoft.com/software-download/windows11arm64 as
`~/VMs/iso/Win11_arm64_en-us.iso`, then run `bash scripts/win-golden.sh`. It installs Windows
unattended (about 30 minutes) and shuts the VM down. Each check then clones it, so the golden VM
stays clean.

## Windows

`./dotfiles` runs on Windows in Git Bash (MSYS/MINGW). Windows is detected with `is_windows`
(`scripts/setup-lib.sh`); WSL counts as Linux and follows the Linux path.

- **Packages:** no Homebrew on Windows. Two steps run there: `12-tools` (mise through WinGet
  `jdx.mise`, then the tools of `tools.list` marked for Windows, from mise first; WinGet and Scoop
  are fallback sources: Opencode `SST.opencode`, yt-dlp, ffmpeg, rustup) and `57-claude-code`
  (`Anthropic.ClaudeCode`, then the PowerShell installer). The shell-only tools (starship, atuin,
  zoxide, eza, yazi, bottom and others) are for macOS and Linux only; zsh does not run on Windows.
  mise and its shims go on PATH for the run; open a new Git Bash afterwards.
- **Links:** real symlinks, so Windows Developer Mode must be on (the script stops with the fix).
  `.config/gh/config.yml`, the Claude config under `~/.claude` and the VS Code and Cursor settings
  (`%APPDATA%\{Code,Cursor}\User\`, only with the `editor` look) link. `~/.gitconfig` is not linked; the git identity goes to
  `~/.gitconfig.local`, which your own `~/.gitconfig` includes.
- **Skipped on Windows:** `70-macos-defaults`, `72-touch-id-sudo` and `75-terminal-appearance`
  print nothing (`dots -v` shows `macOS only, skipping`). The Claude desktop app offer, AeroSpace and the Nerd Font are macOS-only.
- **Shell:** Git Bash provides `zsh`, `git`, `ln -s` (symlinks) and `clip.exe`. `init.zsh` loads the
  usual tools when they are on PATH.
- **Opencode config:** `scripts/agent-config opencode` generates agents, permissions and MCP under
  `~/.config/opencode` and uses `clip` for the clipboard. The output is the same on all three OSes.
- **Python:** not needed. `scripts/agent-config` builds and runs the Rust version. Without `cargo`
  the generated config steps warn and skip, and `dots` installs Rust on its next run.
