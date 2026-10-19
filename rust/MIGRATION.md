# Migration notes: dotfiles Python tooling -> Rust

Status as of 2026-10-16. Goal: Windows machines need no Python.

## State

Every Python tool now has a Rust binary in `rust/agent-config/src/bin/` (or `main.rs`):

| Python | Rust binary | Checked how |
|---|---|---|
| `scripts/agent-config.py` | `agent-config` | `tools/parity_cases.py` 39/42 (the 3 are `--help` wording), `tools/parity_scenarios.py` 6/6 |
| `claude/hooks/guard.py` | `guard` | the 52 cases of `test_guard.py`, run against the binary: 52/52 |
| `cursor/hooks/guard_cursor.py` | `guard_cursor` | the 23 cases of `test_guard_cursor.py`, run against the binary: 23/23 |
| `ps_spawn/scripts/spawn.py` | `spawn` | 420 differential cases with a fake `tmux`/`claude`: 0 diffs |
| `ps_whatsleft/scripts/route.py` | `route` | 1616 differential cases: 2 diffs (inputs where Python itself crashes) |
| `ps_harness-optimize/scripts/harness_stats.py` | `harness_stats` | byte-identical `collect` on 266 real sessions, 17 reader commands |
| `ps_whatsleft/scripts/whatsleft-scan.py` | `whatsleft_scan` | 205 runs, 0 diffs |
| `ps_whatsleft/scripts/hurry-scan.py` | `hurry_scan` | 82 runs, 0 diffs |
| a private Python tool (source outside this repo; it keeps its own copy) | `yt_transcribe`, called by `ps_yt-learn/scripts/fetch.sh` | `--print-id` on 106 links: 0 diffs; 23 unit tests; live captions and Whisper runs. Captions use `yt-dlp` and Whisper uses `whisper-cli` (whisper.cpp), not the Python libraries |

`scripts/agent-config` is the entry point. It runs the Rust build (made on first use when
`cargo` works, rebuilt when `lib.rs`, `main.rs` or `Cargo.toml` change) and falls back to
`agent-config.py`. `AGENT_CONFIG_IMPL=python|rust` forces one. `dotfiles` and
`scripts/install.d/60-cursor.sh` call the shim.

## Known gaps (accepted)

- The Python files stay. They are the reference for the tests and the fallback when there is no
  `cargo`. Delete them only after the hooks and skills call the Rust binaries.
- The Rust `guard` matches `guard.py` as of 2026-10-16 only. The 2026-10-19 changes (model router,
  pace guard via `claude/hooks/pace.py`, plan-mode gate, helper cap, general-purpose refusal, 350K ceiling,
  the `GUARD_OFF` switch, Stop checks G10 to G12, tool checks G13 to G15 with the `posttool` mode, the
  new G3 note and the G6 skip of a deleted hand-off) exist only in Python. Port them, and `pace.py`, before the hooks switch to the binary.
- `claude/settings.json` still wires the Python `guard.py`, and the skills still call their
  `.py` scripts. Moving them to the Rust binaries is a separate, reversible step.
- Where Python crashes with a traceback (null JSON where a number is expected, a lone surrogate,
  a non-UTF-8 prompt file) Rust prints a short error or a safe value instead.
- `--help` and usage text come from clap, not argparse.
- `guard_cursor` reads Cursor's context size by running the `sqlite3` command line tool (read-only,
  1 s timeout). Without that tool (a plain Debian or Ubuntu) it reads 0 and never denies on app
  context; the transcript path still works. Python uses its built-in sqlite3 module.
- Local-time code in the scans uses `localtime_r` on unix and UTC on Windows.
- Rust is tested on Linux: on 2026-10-19 the x1 box (Debian 13, now with `cc`) built it and passed
  `cargo test` (90 tests). Windows is still untested.

## How to run the checks

```sh
cargo build --release --manifest-path rust/agent-config/Cargo.toml
cargo test  --release --manifest-path rust/agent-config/Cargo.toml
python3 scripts/test_agent_config.py                      # 192 Python reference tests
python3 rust/agent-config/tools/parity_cases.py           # needs the release binary
python3 rust/agent-config/tools/parity_scenarios.py       # needs the release binary
python3 rust/agent-config/tools/run_hook_tests.py         # old guard tests vs the Rust hooks (52 + 23)
bash scripts/test-dotfiles.sh
```

`run_hook_tests.py` copies the two Python hook tests to a temp folder and swaps the
interpreter-and-script call for the Rust binary.

## Trap for the next session

`claude-settings promote` writes into the repo. The Python version finds the repo from
`__file__`, so it ignores `DOTFILES_REPO`. To test promote you must copy `claude/`, `cursor/`
and `scripts/` into a temp repo and run the Python script from inside that copy, as
`tools/parity_scenarios.py` does. Otherwise promote edits the real `claude/settings.json`.
