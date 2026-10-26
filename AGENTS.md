# Working in this repo: agent guide

A **plain-git dotfiles repo**: no dotfiles manager, no templating. One script projects it onto the
machine. This is the manual for any AI agent (Claude Code, Cursor, Opencode) that changes it.
Cursor and Opencode read `AGENTS.md` themselves; Claude Code reads it through the repo's
`CLAUDE.md`.

## The one rule

**Edit the source, never `$HOME` by surprise.** Most of `$HOME` is symlinked from `home/` and
`claude/`, so an edit on either side is the same file and is live at once. New files, packages
and machine setup take effect only after `./dotfiles` (alias `dots`: pull, link, provision).

- A managed dotfile (shell, prompt, terminal, git): edit under `home/`.
- Claude config (settings, skills, agents, hooks): edit under `claude/`.
- Unsure whether a `$HOME` file is a link back here: `ls -la`.
- No background sync. This machine pushes; others run `dots` to catch up. Commit and push yourself.

## Layout

Each file's header says what it does. The rows below hold a gotcha.

- `dotfiles`: a shim over the Rust `dots` (`rust/agent-config/src/dots/`). It builds `dots` on first
  use, installs Rust with rustup when missing, and says why if it cannot. `DOTS_BIN=<path>` runs a
  given `dots` (tests). The install steps are in `rust/agent-config/src/dots/install/`, in the order
  of `STEPS` in `install/mod.rs` (`NN-name`).
- `claude/`: Claude config, symlinked per item into `~/.claude`. Rules for skills:
  `claude/skills/README.md`. Core rules: `claude/CLAUDE.md`. Opt-in habits: `claude/presets/`
  (how to add one: `claude/presets/README.md`).
- `claude/hooks/rust-bin`: the launcher for hooks and skills. It runs the Rust bin when it exists,
  with no freshness check, and never builds. With no binary, `--hook` fails open (exit 0, silent)
  and a skill call says "Run: dots" and exits 127.
- `scripts/agent-config`: a shim over the Rust build. With no binary and no `cargo` it says
  "Run: dots" and exits 127. Run `claude/hooks/rust-bin ctx|sessions|proj` for the code reader, the
  session search and the project detector.
- `tools.list`: THE list of command-line tools. Add a tool = add a line. Nothing else lists tools.
- `scripts/setup-lib.sh`: the shell helpers for the few scripts that are still bash.

## Tests

- `cargo test` in `rust/agent-config`: every unit and process test. The pre-commit hook runs
  `rust-bin skill_check` and `cargo test --test skill_bins` for a staged skill change.
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
  `DOTFILES_STEPS` filters the install steps. The pull, git identity prompt, links, skill sync and
  the Opencode config still run: that one is in `dots` itself, not a step.
- `dots uninstall` (`rust/agent-config/src/dots/uninstall.rs`, the Cursor and Opencode part in
  `src/undo.rs`, test `cargo test --test uninstall`): for each new thing `dots` writes or merges,
  add its undo to `undo.rs` and a line to the test.
- `dots doctor` (`rust/agent-config/src/dots/doctor.rs`): `bins-fresh` is the one place a stale
  binary shows, because `rust-bin` never checks. New check: add it to `checks()` and give it a pass
  test and a fail test in `tests/doctor.rs`.
- A new or edited agent (`claude/agents/*.md`) loads only in a new session: test it with a
  headless `claude -p` run, not in the session that edited it (`claude/agents/README.md`).
- Guard checks have an off switch: `GUARD_OFF="G10,G13"` in the env turns those checks off
  (the doc comment in `rust/agent-config/src/bin/guard.rs` lists every check).
- `claude/hooks/` is live in every session. Merge helper branches that touch it in a worktree
  and bring main forward only when the tests pass: a conflict marker breaks every session's
  hooks, and a broken SubagentStop leaves helper slots taken until they time out (90 min).
- `agent-config claude-settings promote` writes into the repo (`claude/settings.json`). It finds
  the repo from `DOTFILES_REPO`, else from the binary's folder. To test it, point `DOTFILES_REPO`
  at a temp copy of `claude/`, `cursor/` and `scripts/`, never at the real checkout.

## Known gaps

Accepted for now:

- `cargo test` runs on the Mac. On Linux and Windows, `scripts/os-check.sh` runs `./dotfiles`,
  not the tests, so the guard is untested on Windows. `GUARD_PLATFORM` (`darwin|linux|win32`)
  stands in for the platform in the macOS-only guard checks (a test hook), and G14 and G15
  assume `/` in paths.
- Local time (the `pace` budget day, the scans) is UTC on Windows unless `TZ` is set: jiff's
  system time zone links windows-link, which crashes next to tantivy's winapi there.

## Skills

`dots` links each `claude/skills/<name>` into `~/.claude/skills`, never touching skills it does not
own (on a name clash the local one wins). Cursor and Opencode read `~/.claude/skills` natively;
nothing goes into `~/.cursor/skills`. New skill: copy `claude/skills/_template/`, keep folder name
== `name:`, run `dots`.

## Cursor

`claude/` is the source for Cursor too: edit it, then run `dots`. Step `60-cursor` runs only when
Cursor is present. What Cursor gets: `docs/ai-agents.md` "Cursor" and `docs/cursor-port.md`.

- Generated Markdown carries a `generated-by: dotfiles` marker. A file without it is the user's.
- Never edit the generated files under `~/.cursor`; the next `dots` overwrites them.
- Never symlink `~/.cursor/cli-config.json` or `~/.cursor/mcp.json`. Cursor rewrites them.
- Rules in every folder: run `cursor-rules`, paste in Cursor Settings > Rules > User Rules.
  `dots` then skips the file rule and warns when to paste again.
- superlite follows the Claude Code switch. `DOTS_SUPERLITE_REPO` points its clone at another repo (tests).

## Opencode

Opencode uses the same `claude/` source: edit it, then run `dots`. `agent-config opencode` runs in
`./dotfiles` itself, not as a step, so `dots settings` runs it too. Skills need no step.

- A generated file carries the `generated-by: dotfiles` marker. A file without it, and a
  `tui.json` that names another theme, is the user's and stays.
- A Claude `model:` pin is dropped from the agents (Opencode names models `provider/model-id`).
- The guard plugin `plugins/dots-guard.ts` is generated from
  `rust/agent-config/src/opencode_plugin.ts.tmpl`. Edit the template, never the output.
- What each piece holds and which guard checks run there: [`docs/ai-agents.md`](docs/ai-agents.md)
  "Opencode". Mapping and gaps: [`docs/opencode-port.md`](docs/opencode-port.md).

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

- Tools come from `tools.list` (step `12-tools`, `scripts/tools`): mise first. The system package
  manager installs only `LINUX_PREREQS` (`scripts/linux-prereqs.txt`, step `05-linux-prereqs`) and,
  on Alpine only, the `apk:` sources of `tools.list`. Never add a system install of tools
  anywhere else.
- Editor settings keep their macOS path under `home/Library/Application Support/` and link to
  `~/.config/{Code,Cursor}/User` on Linux.

## Windows

`./dotfiles` runs in Git Bash (MSYS/MINGW); WSL counts as Linux. Only `12-tools` and
`57-claude-code` run there. What users see: [`docs/platforms.md`](docs/platforms.md#windows).

- **Links:** Git Bash `ln -s` copies unless `MSYS=winsymlinks:nativestrict` (set in `setup-lib.sh`), and a
  native link needs Developer Mode. `./dotfiles` checks this first and stops with the fix. `~/.gitconfig`
  is not linked (it sets `delta` and `code`): the git identity goes to `~/.gitconfig.local`, which the
  user's own `~/.gitconfig` includes.
- **Clone-safe paths:** a tracked path with `: < > " | ? *` or `\`, a trailing dot or space, or a device
  name (`con`, `nul`, ...) makes `git clone` fail on Windows ("invalid path") and leaves a clone with no
  checkout. So a skill folder is `ps_foo` while its `name:` stays `ps:foo`. The pre-commit hook and
  `scripts/test-dotfiles.sh` reject such paths. `.gitattributes` forces LF, and `bootstrap.sh` clones with
  `core.autocrlf=false` and repairs a clone that has no `./dotfiles` (a re-run of the one-liner fixes it).
- **Rust:** `./dotfiles` runs the Rust `dots` here. With no Rust it installs rustup with the
  `x86_64-pc-windows-gnu` host (no Visual Studio Build Tools; Windows on Arm runs it under x64
  emulation) and `llvm-tools`: that build needs LLVM's dlltool, which the shim copies to
  `~/.cargo/dlltool` (`win_dlltool` in `setup-lib.sh`, only for a `-windows-gnu` host: a machine that
  already has the MSVC toolchain builds with it and needs none). It runs a copy of `dots.exe`, because
  Windows locks a running `.exe`. Python is not needed.

## Profiles

`dotfiles_profile` picks one per machine: `DOTFILES_PROFILE`, else `PROFILE` in the settings, else
`owner` when `~/.config/dotfiles/owner.md` exists (saved to the settings), else `shared`.
`DOTFILES_PROFILE=owner` alone is not saved.

- **owner:** `owner.md` (plain Markdown, never in git) is linked as `~/.claude/rules/owner.md`,
  added to the Cursor global rule and written into `~/.config/opencode/AGENTS.md` (0600).
  Without it, `dots` warns.
- **Claude settings**, every profile: `dots` builds `~/.claude/settings.json` (a real file) from
  `claude/settings.json` plus the private layer `~/.config/dotfiles/claude-settings.json`; live
  changes move into the layer first (`scripts/agent-config claude-settings build`).
  `claude-settings promote <key>` moves a key to the repo base. Owner-only keys stay in the layer.
- **Presets** (`claude/presets/`) are enabled by the `PRESETS` setting. A preset that is off costs
  no tokens, and the guard runs a check only while its preset is on. An empty `PRESETS`: `owner`
  gets every preset, `shared` every one but `ship-to-main`.
- Personal rules go in `owner.md` (`dots settings`), never in `~/.claude/CLAUDE.md`: that is the
  repo file, and a local edit can stop the pull.

## Settings

`~/.config/dotfiles/settings` (one `NAME="value"` per line, mode 600, never in git) holds this
machine's settings; `settings.example` lists each with a note and default. `dots settings` shows a
menu that saves each change at once; `dots` then refreshes links, the Cursor rule and the Opencode
config.

- The last line of a name wins. Env vars win for one run: `DOTFILES_PROFILE`, `DOTFILES_MCP`,
  `DOTFILES_STEPS`, `DOTFILES_PROJECTS`.
- Cursor and Opencode MCP servers (`cursor/mcp.json`) are opt-in: `CURSOR_MCP` and `OPENCODE_MCP`.
- Secrets never go in the settings; they stay in the ignored `~/.config/zsh/local.zsh`.

## Conventions

- **Public and impersonal.** Never commit secrets (use `~/.config/zsh/local.zsh`,
  `~/.gitconfig.local`, `Brewfile.local`). The tracked tree names no person: no names, emails,
  accounts, private project names or home-folder paths. One exception: the GitHub handle in the
  default clone URL (`DOTFILES_REPO` in `bootstrap.sh`, the README one-liner). Session notes,
  hand-offs and ledgers stay in the ignored `.agents/`.
- **Private words:** the git hooks, the pre-push hook (it checks every pushed commit and stops a
  push that brings a second history), `dots doctor`
  and guard G16 block the words in `PRIVATE_PROJECTS` and `PRIVATE_WORDS`, the git name and email,
  and home paths. Whole hosts in `PUBLIC_HOSTS` are exempt; paths never are.
- **Story page:** https://stevanpavlovic.com/work/dotfiles tells this repo's story and is the GitHub
  homepage of the repo. It reads the repo
  from GitHub at build: the paths `claude/skills/ps_*/SKILL.md`,
  `claude/agents/*.md`, `claude/presets/*/rules.md`, `claude/output-styles/*.md`, `home/**`,
  `claude/settings.json`, `rust/agent-config/Cargo.toml` and the quoted check ids in
  `rust/agent-config/src/guard_checks.rs`. Moving one of them breaks the page build on purpose.
  A new or renamed skill needs an entry in the page's skill catalog (in the site repo), or it shows as new.
- **Theme:** warm Claude copper-on-stone. Refine it; never swap palettes.
- **Idempotent and safe:** steps detect and skip what is installed, back up config before
  overriding, and warn and continue on failure.
- **Short output:** a run with nothing to do prints about 20 lines, one per step. New step helpers
  are in `rust/agent-config/src/out.rs`: `skip` for "already there", `uptodate` or
  `ok_if_changed` for a closing line that only says "fine", `note` for "nothing to do here".
- **Explicit linking:** `./dotfiles` links only the paths in its `LINKS` array. A new managed file
  means a new entry there.
