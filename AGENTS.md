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

| Path | What |
|------|------|
| `dotfiles` | the one command: git pull, link files, run provisioning steps. A shim: it runs the Rust `dots` (`rust/agent-config/src/dots/`, built on first use; with no Rust it installs one with rustup; on Windows the GNU host) and says why if it cannot. `DOTS_BIN=<path>` runs a given `dots` (tests). |
| `home/` | mirrors `$HOME` 1:1; only paths in the `LINKS` array of `dotfiles` are touched. |
| `rust/agent-config/src/dots/install/` | the provisioning steps, one Rust file each, in the order of `STEPS` in `install/mod.rs` (`NN-name`). Skip-aware and fault-tolerant. |
| `scripts/os-check.sh`, `scripts/win-golden.sh` | the manual check on all three OSes; the one-time builder of the Windows test VM `win11-golden` (UTM, Mac only). |
| `scripts/linux-prereqs.txt` | the Linux packages per manager (apt-get, dnf, pacman, apk, other), read by the shim and by `dots`. |
| `scripts/setup-lib.sh` | shell helpers for the few scripts that are still bash (`dotfiles`, `scripts/tools`, `scripts/agent-config`, `os-check.sh`, `win-golden.sh`, the private-words hook): `info`/`ok`/`warn`/`have`, `is_macos`/`is_linux`/`is_windows`/`is_musl`, `linux_prereqs`, `setting`, `with_heartbeat` (runs a silent build and says "still building" every 20 s, naming a process that holds the cargo lock; used by both Rust builds), `ensure_rust_bin` (find or install Rust, build one bin; used by `dotfiles` and `scripts/tools`). |
| `claude/` | Claude config, symlinked per item into `~/.claude`. Skills in `claude/skills/` (rules: `claude/skills/README.md`). The core rules are `claude/CLAUDE.md`; the opt-in habits are `claude/presets/<name>/` (`rules.md`, optional `settings.json`; list and how to add one: `claude/presets/README.md`). |
| `docs/SETUP-RUNBOOK.md` | the runbook for setting the repo up on a new machine: Claude runs it step by step, then `dots doctor`. |
| `cursor/` | Cursor-only sources: `mcp.json` (merged into `~/.cursor/mcp.json`). The Cursor guard adapter is the Rust bin `guard_cursor`. |
| `scripts/agent-config` | shim: runs the Rust build (`rust/agent-config`, built on first use, rebuilt when anything under `src/`, `Cargo.toml` or `Cargo.lock` changes). With no binary and no `cargo` it says "Run: dots" and exits 127. `--bins` builds every bin and always exits 0. The CLI: `cursor` writes and merges `~/.cursor`; `opencode` writes `~/.config/opencode/{AGENTS.md,agents/,themes/}` and merges permissions/MCP into `opencode.jsonc` and the theme name into `tui.json`; `user-rules` prints Cursor's User Rules text; `claude-settings` builds `~/.claude/settings.json`. |
| `claude/hooks/rust-bin` | launcher for hooks and skills: runs the Rust bin (`.exe` on Windows) when it exists, with no freshness check; never builds. With no binary, `--hook` fails open (exit 0, silent) and a skill call says "Run: dots" and exits 127. |
| `docs/` | the user docs, one page per question (index: `docs/README.md`); `cursor-port.md` and `opencode-port.md` are the Cursor and Opencode mapping and gap tables. `README.md` is the start page. |
| `tools.list` | THE list of command-line tools: `name \| commands \| OSes \| sources in order` (mise, apk, winget, scoop, cargo, brew, sh). Add a tool = add a line. Nothing else lists tools. |
| `scripts/tools`, `scripts/tools-env.sh` | `scripts/tools` is a shim over the Rust bin `tools` (`rust/agent-config/src/tools_core.rs`, `tools_cli.rs`, `bin/tools.rs`). It finds Rust, or installs it with rustup, builds the bin when it is missing or stale (`ensure_rust_bin` in `scripts/setup-lib.sh`, the same code as `./dotfiles`), then runs it. The bin installs mise first when needed, then the tools. `--check`, `--only a,b`, `--list`, `-q`, `-v`. `TOOLS_BIN=<path>` runs a given bin (tests). `tools-env.sh` puts the mise, cargo and WinGet dirs on PATH. |
| `ctx` (`rust/agent-config/src/bin/ctx.rs`, `src/codeview.rs`) | the code reader for agents: `ctx outline FILE` lists symbols with line ranges, `ctx read FILE SYMBOL` prints one. A line scan, the same on every OS. Run it as `claude/hooks/rust-bin ctx`. The guard hint H1 points agents at it when Bash runs `sed -n` or `cat` on a code file. |
| `sessions` (`rust/agent-config/src/bin/sessions.rs`, `src/stores/`) | search past Claude Code sessions: `sessions index`, `sessions find WORDS` (a tantivy index in the user cache dir). Run it as `claude/hooks/rust-bin sessions`; the `session-finder` agent and `ps:sessions` use it. |
| `proj` (`rust/agent-config/src/proj.rs`, `src/bin/proj.rs`) | the project detector: `proj detect [path]` prints the root, package manager, tools, the project's `check`/`check:fast`/`fix` contract and the fast and full check commands; `proj format` is the format-on-edit hook (PostToolUse on Edit, Write, MultiEdit). It also holds the check-once helpers the guard rule W5 uses. The contract and the rules: `claude/hooks/README.md` ("Project contract"). |
| `Brewfile` | the macOS apps only: casks, the tap, the font, `vscode` lines (per-machine extras in the ignored `Brewfile.local`). |
| `settings.example` | every setting with a note and default. The `dots settings` menu is `rust/agent-config/src/dots/menu.rs`. |
| `scripts/test-linux.sh` | runs `./dotfiles` twice in a fresh Linux container (Debian, Ubuntu, Fedora, Arch, Alpine). |
| `bootstrap.sh` | fresh-machine one-liner (macOS, Linux, Windows). The README "Set up" sections have it. |

## Tests

- `cargo test` in `rust/agent-config`: every unit and process test (guard, pace, guard_cursor,
  agent-config, the skill bins), the tool resolver (`tools_core.rs`, fake sources) and a parse of the
  real `tools.list`. The pre-commit hook runs `rust-bin skill_check` and `cargo test --test
  skill_bins` for a staged skill change. No test needs Python.
- `dots uninstall` (`rust/agent-config/src/dots/uninstall.rs`, the Cursor and Opencode part in `src/undo.rs`, test `cargo test --test uninstall`): removes our links, generated files and merged entries, keeps the user's. New thing `dots` writes or merges: add its undo to `undo.rs` and a line to the test.
- `dots doctor` (`rust/agent-config/src/dots/doctor.rs`, test `cargo test --test doctor`): one
  `PASS` or `FAIL` line per check, exit 1 on any FAIL, changes nothing. Checks: `links`,
  `settings-built`, `bins-fresh`, `guard-selftest`, `presets`, `generated`, `timezone`, `no-leak`.
  `bins-fresh` is the one place a stale binary shows: `claude/hooks/rust-bin` runs whatever bin
  exists and never checks it. A FAIL line names the fix (`Run: dots`). New check: add it to
  `checks()` and give it a pass test and a fail test in `tests/doctor.rs`.
- `bash scripts/test-dotfiles.sh`: `./dotfiles` in a temp HOME with a temp repo and origin (first
  run, owner profile, owner rules linked and unlinked, a no-change second run, restart after a
  pull, the warning count on the last line, apt, dnf, pacman and apk faked on a fake Linux, the
  os-check file map, and `scripts/tools` with a fake mise). It builds the Rust `dots` and `tools`
  once and runs the checks with them. About 20
  seconds, nothing installs. On a Mac with Homebrew it also checks step 20 (a satisfied and a failing
  Brewfile); `DOTS_TEST_NETWORK=1` adds step 50 (downloads Node).
- `scripts/os-check.sh` (or `/os-check` in Claude Code): the manual check on all three OSes, run
  before a commit to setup code. No git hook and no CI runs it. It checks each OS whose files
  changed (`--force` runs one anyway, `--since <ref>` widens the diff): macOS with
  `test-dotfiles.sh`, Linux with `test-linux.sh` on the `TEST_HOST` SSH host (or local podman or
  docker; `--local` or `TEST_HOST=local` forces local, e.g. when the host is off), Windows in a throwaway clone of the UTM VM `win11-golden` (built once with
  `scripts/win-golden.sh`; see "Windows"). The OSes run at the same time, and each result line
  prints when its OS ends. Logs: `.os-check/` (Linux per
  image: `.os-check/linux-logs/`). Each run has its own container names and host folder, so two
  worktrees can run it at once. `--which FILE...` prints the OSes a file is for. A new path rule
  needs a line in the map test of `test-dotfiles.sh` (section 12).
- `scripts/test-linux.sh` also fails on shell errors that skip `warn` (`command not found`,
  `unbound variable`, a Rust panic), on a last line that is not the done line,
  on a second run over 90 seconds or one that rebuilds the Rust tools, and on a failed `dots -v`.
  Its base image tag holds a hash of the recipe, so a recipe change builds a fresh image.
- One step in a temp HOME: `HOME=$(mktemp -d) DOTFILES_STEPS=60-cursor ./dotfiles`.
  `DOTFILES_STEPS` filters the install steps (names from `STEPS`; empty runs all). The pull, git
  identity prompt, links, skill sync and the Opencode config step still run: that one is in
  `dots` itself, not a step, so it needs no filter.
- A new or edited agent (`claude/agents/*.md`) loads only in a new session: test it with a
  headless `claude -p` run, not in the session that edited it (`claude/agents/README.md`).
- Guard checks have an off switch: `GUARD_OFF="G10,G13"` in the env turns those checks off
  (the `GUARD_OFF` doc comment in `rust/agent-config/src/bin/guard.rs` lists every check).
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
- `pace` skips a JSON NaN line or a lone surrogate in a transcript and ignores an unwritable
  cache dir. Local time (the `pace` budget day, the scans) is UTC on Windows unless `TZ` is set:
  jiff's system time zone links windows-link, which crashes next to tantivy's winapi there.
- `guard_cursor` reads Cursor's context size with the `sqlite3` command (read-only, 1 s
  timeout). Without it (a plain Debian or Ubuntu) it reads 0 and never denies on app context;
  the transcript path still works.
- `route` names itself `route.py` in its messages, as the old Python tool did.

## Skills

`dots` (`rust/agent-config/src/dots/skills.rs`) links each `claude/skills/<name>` into `~/.claude/skills`, never touching
skills it does not own (on a name clash the local one wins). Cursor and Opencode read
`~/.claude/skills` natively; nothing goes into `~/.cursor/skills` (and Opencode has no skills
dir of its own). New skill: copy `claude/skills/_template/`, keep folder name == `name:`, run
`dots`.

## Cursor

`claude/` is the source for Cursor too: edit it, then run `dots`. Step `60-cursor` runs only when
Cursor is present: it installs the CLI, the Brewfile `vscode` extensions, and runs
`scripts/agent-config cursor`. What Cursor gets: `docs/ai-agents.md` "Cursor" and
`docs/cursor-port.md`.

- Generated Markdown carries a `generated-by: dotfiles` marker. A file without it is the user's.
- Never edit the generated files under `~/.cursor`; the next `dots` overwrites them.
- Never symlink `~/.cursor/cli-config.json` or `~/.cursor/mcp.json`. Cursor rewrites them.
- Rules in every folder: run `cursor-rules`, paste in Cursor Settings > Rules > User Rules.
  `dots` then skips the file rule and warns when to paste again.
- superlite follows the Claude Code switch (`superlite@superlite-dev` in the built
  `~/.claude/settings.json`): a clone in `~/.cursor/plugins/local/superlite` and the spec in
  Opencode's `plugins`. `DOTS_SUPERLITE_REPO` points the clone at another repo (tests).

## Opencode

Opencode (the `opencode` CLI and the Opencode app) uses the same `claude/` source: edit it, then
run `dots`. Step `58-opencode` (`dots/install/opencode.rs`) checks the `opencode` CLI.
`agent-config opencode` runs in `./dotfiles` itself, so `dots settings` runs it too. It writes
`AGENTS.md` (the rules with the owner overlay and the output style), `agents/`, the copper theme
and the guard plugin `plugins/dots-guard.ts` (source `rust/agent-config/src/opencode_plugin.ts.tmpl`)
under `~/.config/opencode/`, and merges the permissions and MCP servers into `opencode.jsonc`.
Skills need no step: Opencode reads `~/.claude/skills`.

- A generated file carries the `generated-by: dotfiles` marker. A file without it, and a
  `tui.json` that names another theme, is the user's and stays.
- A Claude `model:` pin is dropped from the agents (Opencode names models `provider/model-id`).
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
  `scripts/os-check.sh windows` for Windows (a UTM VM on a Mac; see "Windows" below).

## Linux

`./dotfiles` runs on Debian, Ubuntu, Fedora, Arch and Alpine (a normal user with `sudo`). No Homebrew
on Linux. What users see: [`docs/platforms.md`](docs/platforms.md#linux).

- Tools come from `tools.list` (step `12-tools`, `scripts/tools`): mise first, installed by its official
  script (musl too). The system package manager (apt, dnf, pacman or apk: `LINUX_PKG`) installs only
  `LINUX_PREREQS` (`scripts/linux-prereqs.txt`, step `05-linux-prereqs`) and, on Alpine only, the `apk:`
  sources of `tools.list`. Never add a system install of tools anywhere else.
- Paths: `scripts/tools-env.sh` puts `~/.local/bin`, the mise shims and `~/.cargo/bin` on PATH for every
  step. Editor settings keep their macOS path under `home/Library/Application Support/` and link to
  `~/.config/{Code,Cursor}/User` on Linux.
- Test: `bash scripts/test-linux.sh debian:13` (also `ubuntu:24.04`, `fedora:latest`,
  `archlinux:latest`, `alpine:latest`) exits 0 and the second run changes nothing. Needs podman or
  docker. `--keep` leaves the container, `--reuse` reruns your edits.

## Windows

`./dotfiles` runs in Git Bash (MSYS/MINGW), detected with `is_windows`; WSL counts as Linux. What users
see: [`docs/platforms.md`](docs/platforms.md#windows).

- **Steps:** only `12-tools` and `57-claude-code` run. Homebrew, zsh and the Brewfile do not exist
  there. `DOTFILES_STEPS` still runs any step by name.
- **Links:** Git Bash `ln -s` copies unless `MSYS=winsymlinks:nativestrict` (set in `setup-lib.sh`), and a
  native link needs Developer Mode. `./dotfiles` checks this first and stops with the fix. `~/.gitconfig`
  is not linked (it sets `delta` and `code`): the git identity goes to `~/.gitconfig.local`, which the
  user's own `~/.gitconfig` includes. Links print one total line. The first run skips the settings menu.
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
- **Test VM (Mac only):** `scripts/os-check.sh windows` clones the UTM VM `win11-golden`
  (Windows 11 Arm64, OpenSSH Server, Developer Mode, Git, PowerShell 7), starts the clone,
  copies the tree over SSH, runs `./dotfiles` twice in Git Bash, then once more after a pull that
  brings a Rust change (the restart rebuilds dots.exe), checks the exit codes, the links and
  `pwsh -Command exit`, runs `dots uninstall -y` twice (the links go, the repo stays), then deletes
  the clone, also after a failure. It never starts
  the golden VM. Build the golden VM once with `bash scripts/win-golden.sh` (needs UTM and the
  Windows ISO in `~/VMs/iso`; the script reads your SSH key at run time). The VM, its answer
  file and its login stay in `~/VMs`, outside the repo. A start failure (`utmctl` exit 142, UTM's
  stuck Apple Events) makes the script quit and reopen UTM and try again, unless a VM is running. If it
  still fails: quit and reopen UTM by hand, delete the leftover `win11-check-*` clone, run it again.

## Profiles

`dotfiles_profile` picks one per machine: `DOTFILES_PROFILE`, else `PROFILE` in the settings, else
`owner` when `~/.config/dotfiles/owner.md` exists (saved to the settings), else `shared`.
`DOTFILES_PROFILE=owner` alone is not saved.

- Every profile links `~/.claude/CLAUDE.md` to `claude/CLAUDE.md`. The repo holds nothing personal.
- **owner:** `owner.md` (plain Markdown, never in git) is linked as `~/.claude/rules/owner.md`,
  added to the Cursor global rule and written into `~/.config/opencode/AGENTS.md` (0600).
  Without it, `dots` warns. Check: `rg -n '^## ' ~/.claude/rules/owner.md`.
- **shared:** no overlay; plugins in `claude/settings.json` are shared as is.
- **Claude settings**, every profile: `dots` builds `~/.claude/settings.json` (a real file) from
  `claude/settings.json` plus the private layer `~/.config/dotfiles/claude-settings.json`; live
  changes move into the layer first (`scripts/agent-config claude-settings build`).
  `claude-settings promote <key>` moves a key to the repo base. Owner-only keys stay in the layer.
- **Presets:** the habits that are not core rules (`claude/presets/`, seven of them). The
  `PRESETS` setting picks them: `all`, `none`, or names. Empty: `owner` gets every preset, `shared`
  every one but `ship-to-main` (a push to a work repo's main can hurt). `dots` links each enabled
  preset to `~/.claude/rules/preset-<name>.md`, merges its `settings.json` fragment between the repo
  base and the private layer, and the guard runs a check only while its preset is on. A preset
  that is off costs no tokens. `dots doctor` checks that the links equal the setting.
- **Cursor MCP servers** (`cursor/mcp.json`) are opt-in: `CURSOR_MCP="on"`, or `DOTFILES_MCP=on` once.
- **Opencode MCP servers** (the same `cursor/mcp.json`) are opt-in: `OPENCODE_MCP="on"`, or
  `DOTFILES_MCP=on` once.
- **Opencode projects:** `OPENCODE_PROJECTS` (space-separated paths, `~` allowed) or
  `DOTFILES_PROJECTS` for one run; each gets a `.opencode/` mirroring the global config.
- Personal rules go in `owner.md` (`dots settings`), never in `~/.claude/CLAUDE.md`: that is the
  repo file, and a local edit can stop the pull.

## Settings

`~/.config/dotfiles/settings` (one `NAME="value"` per line, mode 600, never in git) holds this
machine's settings; `settings.example` lists each with a note and default. `dots settings` (also
`dots private`) shows a menu that saves each change at once; Done ends it and `dots` refreshes
links, the Cursor rule and the Opencode config. The first run (no file) asks git name and
email, then shows the menu.

- Names: `GIT_NAME`, `GIT_EMAIL` (written to `~/.gitconfig.local`), `PROFILE`, `CURSOR_MCP`,
  `OPENCODE_MCP`, `OPENCODE_PROJECTS`, `PRESETS` (which `claude/presets/` are linked and merged;
  `claude/presets/README.md`), `TIMEZONE`, `PLAN`, `WORK_DAYS`, `THEME` (`plain` drops colors and the
  Opencode theme), `LOOKS` (the optional look changes: `starship`, `editor`, `font` or `none`;
  empty asks once at a terminal; `rust/agent-config/src/dots/looks.rs`), `GUARD_OFF`, `GUARD_SHADOW`, `JAVA_HOME` (picked from the Javas found, or typed),
  `SKIP_STEPS`, `PRIVATE_PROJECTS`, `PRIVATE_WORDS` (see "Conventions"), `TEST_HOST` (the SSH host
  for the Linux part of `scripts/os-check.sh`).
- Readers: `setting` in `scripts/setup-lib.sh`, `setting()` in `rust/agent-config/src/lib.rs`,
  `home/.config/zsh/settings.zsh` (sourced after `local.zsh`, so its `JAVA_HOME` wins). The last
  line of a name wins. Env vars win for one run: `DOTFILES_PROFILE`, `DOTFILES_MCP`,
  `DOTFILES_STEPS`, `DOTFILES_PROJECTS`.
- Secrets never go in the settings; they stay in the ignored `~/.config/zsh/local.zsh`.

## Conventions

- **Public and impersonal.** Never commit secrets (use `~/.config/zsh/local.zsh`,
  `~/.gitconfig.local`, `Brewfile.local`). The tracked tree names no person: no names, emails,
  accounts, private project names or home-folder paths. One exception: the GitHub handle in the
  default clone URL (`DOTFILES_REPO` in `bootstrap.sh`, the README one-liner). Session notes,
  hand-offs and ledgers stay in the ignored `.agents/`.
- **Private words:** `PRIVATE_PROJECTS`, `PRIVATE_WORDS`, the git email, the full git name and that
  name as one word (a word in the origin URL is left out). A word of 4 letters or fewer matches
  whole words only. The hooks also block this user's home paths (`/Users/<user>`, `/home/<user>`,
  `C:\Users\<user>`) and warn, without blocking, when the word list is empty. The git hooks block
  a staged file name, added line or commit message that holds one. The pre-push hook checks every pushed commit the
  same way (cherry-pick, rebase and `--no-verify` skip the commit hooks) and stops a push that
  brings a second history.
- **Story page:** the GitHub homepage of this repo is a page that tells its story. It reads the repo
  from GitHub at build: the paths `claude/skills/ps_*/SKILL.md`,
  `claude/agents/*.md`, `claude/presets/*/rules.md`, `claude/output-styles/*.md`, `home/**`,
  `claude/settings.json`, `rust/agent-config/Cargo.toml` and the quoted check ids in
  `rust/agent-config/src/guard_checks.rs`. Moving one of them breaks the page build on purpose.
  A new or renamed skill needs an entry in the page's skill catalog (in the site repo), or it shows as new.
- **Theme:** warm Claude copper-on-stone. Refine it; never swap palettes.
- **macOS, Linux and Windows:** one code path where possible; OS-only branches behind
  `is_macos`/`is_linux`/`is_windows`.
- **Idempotent and safe:** steps detect and skip what is installed, back up config before
  overriding, and warn and continue on failure.
- **Short output:** a run with nothing to do prints about 20 lines on every OS, one per step
  (`• Oh My Zsh + plugins: 5 up to date`). `skip` counts instead of printing; a step header waits
  until the step prints something; a written, changed or backed-up file, a warning and an error
  still print in full. `dots -v` (or `DOTFILES_VERBOSE=1`) prints everything. The helpers live in
  `rust/agent-config/src/out.rs`. New step: use `skip` for "already there", `uptodate` or
  `ok_if_changed` for a closing line that only says "fine", `note` for "nothing to do here". The
  git hooks and `scripts/test-dotfiles.sh` print only failures (`test-dotfiles.sh -v` shows each
  pass).
- **Explicit linking:** `./dotfiles` links only the paths in its `LINKS` array. A new managed file
  means a new entry there.
- **New machine:** `bootstrap.sh` installs Xcode CLT (macOS) or git and curl (Linux), clones
  `DOTFILES_REPO` and runs `./dotfiles`.
  At a terminal the first run is guided (gum menus: git identity, settings, an optional extra, the
  sign-ins in `90-sign-in.sh`: GitHub, Claude Code, Opencode, Cursor); without one it reads name and
  email from stdin and skips the rest.
