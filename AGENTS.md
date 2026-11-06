# Working in this repo: agent guide

A **plain-git dotfiles repo**: no dotfiles manager, no templating. One script projects it onto the
machine. This is the manual for any AI agent (Claude Code, Cursor, Opencode) that changes it.
Cursor and Opencode read `AGENTS.md` themselves; Claude Code reads it through the repo's
`CLAUDE.md`.

## The one rule

**Edit the source, never `$HOME` by surprise.** Most of `$HOME` is symlinked from `home/`, so an
edit on either side is the same file and is live at once. New files, packages
and machine setup take effect only after `./dotfiles` (alias `dots`: pull, link, provision).
No background sync. This machine pushes; others run `dots` to catch up. Commit and push yourself.

## Layout

Each file's header says what it does. The rows below hold a gotcha.

- `dotfiles`: a shim over the Rust `dots` (`rust/agent-config/src/dots/`). It builds `dots` on first
  use, installs Rust with rustup when missing, and says why if it cannot. `DOTS_BIN=<path>` runs a
  given `dots` (tests). The install steps are in `rust/agent-config/src/dots/install/`, in the order
  of `STEPS` in `install/mod.rs` (`NN-name`).
- `claude/settings.json`: the base of `~/.claude/settings.json` (built, not linked: the base, the
  `claude/preset-settings/<preset>.json` of the presets that are on, then the private layer).
- The AI harness is not here. Rules, presets, skills, agents, output styles, the guard hooks, the
  status line and the Cursor and Opencode config are the `ps` plugin in
  [pavstev/agent-kit](https://github.com/pavstev/agent-kit). `dots` installs it with the `claude`
  CLI, removes the old links into `claude/`, and runs the plugin's `agent-config` for Cursor and
  Opencode (`rust/agent-config/src/dots/agent_kit.rs`). The plugin keeps `~/.claude/hooks` as a
  link to its binaries, so `~/.claude/hooks/rust-bin` still works.
- `scripts/agent-config`: a shim over the Rust build. With no binary and no `cargo` it says
  "Run: dots" and exits 127.
- `tools.list`: THE list of command-line tools. Add a tool = add a line. Nothing else lists tools.
- `scripts/setup-lib.sh`: the shell helpers for the few scripts that are still bash.

## Tests

- `cargo test` in `rust/agent-config`: every unit and process test.
- `bash scripts/test-dotfiles.sh`: `./dotfiles` in a temp HOME with a temp repo and origin. About
  20 seconds, nothing installs.
- `scripts/os-check.sh` (or `/os-check` in Claude Code): the manual check on all three OSes, run
  before a commit to setup code. No git hook and no CI runs it. It checks each OS whose files
  changed (`--force` runs one anyway, `--since <ref>` widens the diff). Linux runs on the
  `TEST_HOST` SSH host, or local podman or docker (`--local` or `TEST_HOST=local` forces local).
  Logs: `.os-check/`. A new path rule needs a line in the map test of `test-dotfiles.sh`
  (section 12).
- `bash scripts/test-linux.sh debian:13` (also `ubuntu:24.04`, `fedora:latest`, `archlinux:latest`,
  `alpine:latest`): exits 0 and the second run changes nothing. `--keep` leaves the container,
  `--reuse` reruns your edits. It also fails on a shell error that skips `warn`, a Rust panic, a
  last line that is not the done line, and a second run over 90 seconds or one that rebuilds.
- One step in a temp HOME: `HOME=$(mktemp -d) DOTFILES_STEPS=60-cursor ./dotfiles`.
  `DOTFILES_STEPS` filters the install steps. The pull, git identity prompt, links, the plugin
  hand-over and the Opencode config still run: those are in `dots` itself, not steps.
  `DOTS_NO_PLUGIN=1` skips the plugin install (tests).
- `dots uninstall` (`rust/agent-config/src/dots/uninstall.rs`, test `cargo test --test uninstall`):
  for each new thing `dots` writes, add its undo and a line to the test. The plugin and the Cursor
  and Opencode config it wrote stay.
- `dots doctor` (`rust/agent-config/src/dots/doctor.rs`): `bins-fresh` is the one place a stale
  binary shows. `plugin` says the ps plugin has its binaries. New check: add it to `checks()` and
  give it a pass test and a fail test in `tests/doctor.rs`.
- `agent-config claude-settings promote` writes into the repo (`claude/settings.json`). It finds
  the repo from `DOTFILES_REPO`, else from the binary's folder. To test it, point `DOTFILES_REPO`
  at a temp copy of `claude/` and `scripts/`, never at the real checkout.

## Known gaps

Accepted for now:

- `cargo test` runs on the Mac. On Linux and Windows, `scripts/os-check.sh` runs `./dotfiles`,
  not the tests.

## Cursor and Opencode

Their rules, agents, hooks and skill links come from the ps plugin
([pavstev/agent-kit](https://github.com/pavstev/agent-kit), its `docs/cursor-port.md` and
`docs/opencode-port.md`). Step `60-cursor` installs the Cursor CLI and the editor extensions and
then runs the plugin's `agent-config cursor`; `dots` runs its `agent-config opencode`. Before the
plugin's first Claude Code session it has no binaries, and both only say so.

- Never symlink `~/.cursor/cli-config.json` or `~/.cursor/mcp.json`. Cursor rewrites them.

## Platforms

`./dotfiles` runs on macOS, Linux and Windows (Git Bash). All three are
first-class: a change must keep each one working. Write docs and messages for all three; name an
OS only where the behavior really differs.

- Ask the OS with `dots::os` (`is_macos`, `is_linux`, `is_windows`) in Rust and the same names
  from `scripts/setup-lib.sh` in shell, never inline `uname`. A macOS-only step starts with
  `if !only_macos("<what>") { return; }`; a Linux-only step with `if !os::is_linux() { return; }`.
- No GNU-only or BSD-only flags in shared scripts (`sed -i`, `stat`, `date -j`, `readlink -f`).
- Tests: `scripts/test-dotfiles.sh` on the machine you work on, `scripts/test-linux.sh` for Linux,
  `scripts/os-check.sh windows` for Windows (a UTM VM on a Mac, built once with
  `scripts/win-golden.sh`). If UTM fails to start (`utmctl` exit 142 or -609), quit and reopen UTM,
  delete the leftover `win11-check-*` clone, and run it again.

## Linux

What users see: [`docs/platforms.md`](docs/platforms.md#linux). No Homebrew on Linux.

- The system package manager installs only `LINUX_PREREQS` (`scripts/linux-prereqs.txt`, step
  `05-linux-prereqs`) and, on Alpine only, the `apk:` sources of `tools.list`. Never add a system
  install of tools anywhere else. How tools install (mise first) is in `docs/platforms.md`.
- Editor settings keep their macOS path under `home/Library/Application Support/`; the Linux
  links are in `docs/platforms.md`.

## Windows

`./dotfiles` runs in Git Bash (MSYS/MINGW). What users see (steps that run, WSL, links, the git
identity file): [`docs/platforms.md`](docs/platforms.md#windows). The gotchas:

- **Links:** Git Bash `ln -s` copies unless `MSYS=winsymlinks:nativestrict` (set in `setup-lib.sh`),
  and a native link needs Developer Mode. `./dotfiles` checks this first and stops with the fix.
  `~/.gitconfig` is not linked because it sets `delta` and `code`.
- **Clone-safe paths:** a tracked path with `: < > " | ? *` or `\`, a trailing dot or space, or a device
  name (`con`, `nul`, ...) makes `git clone` fail on Windows ("invalid path") and leaves a clone with no
  checkout. The pre-commit hook and
  `scripts/test-dotfiles.sh` reject such paths. `.gitattributes` forces LF, and `bootstrap.sh` clones with
  `core.autocrlf=false` and repairs a clone that has no `./dotfiles` (a re-run of the one-liner fixes it).
- **Rust:** the `-windows-gnu` host the shim installs (`docs/platforms.md`; Windows on Arm runs it
  under x64 emulation) builds with LLVM's dlltool, which the shim copies to `~/.cargo/dlltool`
  (`win_dlltool` in `setup-lib.sh`, only for a `-windows-gnu` host: a machine that already has the
  MSVC toolchain needs none). It runs a copy of `dots.exe`, because Windows locks a running `.exe`.

## Profiles

`dotfiles_profile` picks one per machine: `DOTFILES_PROFILE`, else `PROFILE` in the settings, else
`owner` when `~/.config/dotfiles/owner.md` exists (saved to the settings), else `shared`.
`DOTFILES_PROFILE=owner` alone is not saved.

- **owner:** `owner.md` (plain Markdown, never in git) is linked as `~/.claude/rules/owner.md`,
  added to the Cursor global rule and written into `~/.config/opencode/AGENTS.md` (0600).
  Without it, `dots` warns.
- **Claude settings**, every profile: `dots` builds `~/.claude/settings.json` (a real file) from
  `claude/settings.json`, the fragments of the presets that are on, and a private layer;
  `claude-settings promote <key>` is in [`docs/customize.md`](docs/customize.md). Owner-only keys
  stay in the layer.
- **Presets** are the ps plugin's; the `PRESETS` setting turns them on. A preset that needs a
  settings key has a fragment here: `claude/preset-settings/<name>.json`. An empty `PRESETS`:
  `owner` gets every preset, `shared` every one but `ship-to-main`.
- Personal rules go in `owner.md` (`dots settings`), never in the repo.

## Settings

`~/.config/dotfiles/settings` (one `NAME="value"` per line, mode 600, never in git) holds this
machine's settings; `settings.example` lists each with a note and default. `dots settings` shows a
menu that saves each change at once; `dots` then refreshes links, the Cursor rule and the Opencode
config.

- The last line of a name wins. Env vars win for one run: `DOTFILES_PROFILE`, `DOTFILES_MCP`,
  `DOTFILES_STEPS`, `DOTFILES_PROJECTS`.
- Cursor and Opencode MCP servers (from the ps plugin) are opt-in: `CURSOR_MCP` and `OPENCODE_MCP`.
- Secrets never go in the settings; they stay in the ignored `~/.config/zsh/local.zsh`.

## Conventions

- **Public and impersonal.** Never commit secrets (use `~/.config/zsh/local.zsh`,
  `~/.gitconfig.local`, `Brewfile.local`). The tracked tree names no person: no names, emails,
  accounts, private project names or home-folder paths. One exception: the GitHub handle in the
  default clone URL (`DOTFILES_REPO` in `bootstrap.sh`, the README one-liner). Session notes,
  hand-offs and ledgers stay in the ignored `.agents/`.
- **Private words:** the git hooks, the pre-push hook (it checks every pushed commit and stops a
  push that brings a second history), `dots doctor` (`dots/leak.rs`) and the ps plugin's guard
  block the words in `PRIVATE_PROJECTS` and `PRIVATE_WORDS`, the git name and email,
  and home paths. Whole hosts in `PUBLIC_HOSTS` are exempt; paths never are.
- **Story page:** https://stevanpavlovic.com/work/dotfiles tells this repo's story and is the GitHub
  homepage of the repo. It reads the repo from GitHub at build: `home/**`, `claude/settings.json`
  and `rust/agent-config/Cargo.toml`. Moving one of them breaks the page build on purpose. The
  skills, agents, presets, output styles and guard check ids it showed moved to the ps plugin, so
  the page's reader of those paths needs the same move (in the site repo).
- **Theme:** warm Claude copper-on-stone. Refine it; never swap palettes.
- **Idempotent and safe:** steps detect and skip what is installed, back up config before
  overriding, and warn and continue on failure.
- **Short output:** a run with nothing to do prints about 20 lines, one per step. New step helpers
  are in `rust/agent-config/src/out.rs`: `skip` for "already there", `uptodate` or
  `ok_if_changed` for a closing line that only says "fine", `note` for "nothing to do here".
- **Explicit linking:** `./dotfiles` links only the paths in its `LINKS` array. A new managed file
  means a new entry there.
