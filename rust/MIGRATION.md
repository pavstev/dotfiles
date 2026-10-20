# Migration notes: dotfiles Python tooling -> Rust

Status: done. The repo is Rust only. No tracked `.py` file is left, and no test, hook or skill
needs `python3`. Windows machines need no Python.

## State

Every tool that was Python is now a Rust binary in `rust/agent-config/src/bin/` (or `main.rs`):

| Was | Rust binary |
|---|---|
| `scripts/agent-config.py` | `agent-config` (`scripts/agent-config` is the shim) |
| `claude/hooks/guard.py` | `guard` |
| `claude/hooks/pace.py` | `pace` (and `pace_core.rs`, used by `guard`) |
| `cursor/hooks/guard_cursor.py` | `guard_cursor` |
| `ps_spawn/scripts/spawn.py` | `spawn` |
| `ps_whatsleft/scripts/route.py` | `route` |
| `ps_harness-optimize/scripts/harness_stats.py` | `harness_stats` |
| `ps_whatsleft/scripts/whatsleft-scan.py` | `whatsleft_scan` |
| `ps_whatsleft/scripts/hurry-scan.py` | `hurry_scan` |
| `claude/skills/check.py` | `skill_check` |
| `claude/statusline.sh` | `statusline` (no `jq` needed) |
| `ps_mic-drop/scripts/mic-check.sh` | `mic_check` |
| `ps_bag-mode/scripts/bag-check.sh` | `bag_check` |
| `ps_yt-learn/scripts/fetch.sh` and the private transcript tool | `yt_transcribe` (`fetch <url>` prints the cached file) |
| `claude/skill-sync.sh` | the skills step of `dots` (`src/dots/skills.rs`) |

The 461 Python tests were ported to Rust tests. `tests/PORTED.md` maps each old test to its Rust test
(or says why it no longer applies), and `ported_map_complete` in `tests/guard.rs` checks that the map
has all 461 rows and that each named Rust test exists. Keep that file as the record of what was ported.

`scripts/agent-config` is the entry point for `agent-config`. It runs the Rust build (made on first
use when `cargo` works, rebuilt when anything under `src/`, `Cargo.toml` or `Cargo.lock` changes).
With no usable binary and no `cargo` it says "Run: dots" and exits 127. `dotfiles` and step
`60-cursor` call the shim. `scripts/agent-config --bins` builds every bin and always exits 0.

`claude/hooks/rust-bin [--hook] <name> [args]` is the launcher for hooks and skills. It runs
`target/release/<name>` (`.exe` on Windows) when it exists, with no freshness check, so a call costs
no more than the binary. It never builds, so a hook never waits on `cargo`. With no binary, `--hook` exits 0 silently and a
skill call says "Run: dots" (exit 127). `claude/settings.json` (hooks and the status line), the
`ui-verifier` agent hook, the Cursor hooks (`rust-bin --hook guard_cursor <mode>`, written by
`agent-config cursor`) and the skills call it.

## The setup itself: `dots`

`./dotfiles` is a shim for the Rust bin `dots` (`src/dots/`, shared output in `src/out.rs`). It
pulls, links, runs the settings menu (inquire prompts), syncs skills, runs agent-config in
process, and runs the install steps (`src/dots/install/`, listed in `STEPS`). Step 12-tools runs
the tool resolver in process (`src/tools_cli.rs`, `src/tools_core.rs`). There is no bash setup.
With no Rust, the shim installs a minimal stable toolchain with rustup (Linux: the packages of
`scripts/linux-prereqs.txt` first; Windows: the `x86_64-pc-windows-gnu` host with `llvm-tools`,
whose LLVM dlltool the build needs). When that or the build fails, it says why and exits 1. On
Windows the shim runs a copy of `dots.exe`, so the restart after a pull can rebuild it (not yet
covered by `os-check windows`: the throwaway clone has no new commit to pull). Windows prompts use
inquire's console backend, because crossterm crashed `dots.exe` before `main`.
`scripts/test-dotfiles.sh` builds `dots` and runs its checks with it.

## Known gaps (accepted)

- `GUARD_PLATFORM` (`darwin|linux|win32`) stands in for the platform in the macOS-only checks of
  `guard` (a test hook). Paths in G14 and G15 assume `/` separators. Windows is untested.
- `pace`: a JSON NaN line or a lone surrogate in a transcript is skipped, an unwritable cache dir
  is ignored silently, and on Windows the budget day uses UTC.
- `--help` and usage text come from clap.
- `guard_cursor` reads Cursor's context size by running the `sqlite3` command line tool (read-only,
  1 s timeout). Without that tool (a plain Debian or Ubuntu) it reads 0 and never denies on app
  context; the transcript path still works.
- Local-time code in the scans uses `localtime_r` on unix and UTC on Windows.
- Rust is tested on Linux (a Debian 13 x86_64 box with `cc`). Windows is still untested.
- Program names in a few messages (`route.py: bad input`, `spawn.py: error`) are kept as the old
  Python tools printed them, because the ported tests check that text.

## How to run the checks

```sh
cargo build --release --manifest-path rust/agent-config/Cargo.toml
cargo test  --release --manifest-path rust/agent-config/Cargo.toml
bash scripts/test-dotfiles.sh
```

## Trap for the next session

`agent-config claude-settings promote` writes into the repo (`claude/settings.json`). It finds the
repo from `DOTFILES_REPO`, else from the binary's folder. To test it, point `DOTFILES_REPO` at a
temp copy of `claude/`, `cursor/` and `scripts/`, never at the real checkout.
