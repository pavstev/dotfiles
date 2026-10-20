# Benchmark: agent-config

Start-up time is what counts here. Claude Code starts a new process for every hook (each tool
call, prompt and stop) and for each status line redraw. Most bins finish in about 1.5 ms, which is
close to the cost of starting any process. So the wins come from what runs around the bin: the
launcher, a slow scan, a spawned `git`.

## How to re-run

- `cargo bench --bench hooks`: hermetic. It builds a temp HOME with about 200 small transcripts
  and one 5 MB transcript and reads nothing from the real HOME. Knobs: `BENCH_RUNS=<n>`,
  `BENCH_FILTER=<text>`.
- The hook as Claude Code runs it (launcher included), with a saved PreToolUse payload:

  ```sh
  hyperfine -N --warmup 5 --runs 100 --input pre_bash.json "sh -c 'sh ~/.claude/hooks/rust-bin --hook guard pretool'"
  ```

  hyperfine takes one `--input` per run, so run one hyperfine per case.

The numbers below: Apple Silicon Mac, Rust 1.99, `hyperfine -N`, 60 to 100 runs after 5 warmup
runs, mean in ms. Base is commit ef985ab. Other sessions ran at the same time, so read them as
plus or minus 10%.

## The hook launcher

Every hook runs `claude/hooks/rust-bin`, which finds the bin and runs it.

| hook command | ms |
|---|---:|
| before: `bash rust-bin --hook guard pretool` | 15.4 |
| after: `sh rust-bin --hook guard pretool` | 6.1 |
| floor: `sh -c 'exec guard pretool'` | 3.8 |

Why: Homebrew bash takes about 6 ms to start, and the old launcher also forked `uname`,
`dirname` and a `cd` subshell. The new launcher is plain POSIX `sh` with no fork. About 9 ms saved
on each hook call.

## Each bin, started directly

| case | before | after | note |
|---|---:|---:|---|
| guard pretool Bash | 1.82 | 1.81 | |
| guard pretool Agent | 2.64 | 2.42 | |
| guard posttool | 1.51 | 1.48 | |
| guard prompt | 23.36 | 1.99 | rescans transcripts only when the last scan is partial, older than 2 min or from another week; then about 22 ms |
| statusline | 8.14 | 1.71 | reads `.git/HEAD`; runs `git` only as a fallback |
| pace status | 1.61 | 1.72 | |
| guard_cursor tool | 1.74 | 1.79 | |
| agent-config --help | 1.56 | 1.47 | |
| dots --help | 1.77 | 1.67 | |

## Size and build time

| | before | after (strip) | fat LTO, tried and dropped |
|---|---:|---:|---:|
| 16 bins on disk | 39 MB | 31 MB | 26 MB |
| clean release build | 12.8 s | 13.6 s | 30.9 s |
| rebuild after one change in `lib.rs` | 7.9 s | 8.2 s | 29.7 s |

Fat LTO with `codegen-units = 1` made the heavy paths about 15% faster but did not change
start-up time. Each `dots` run after a pull with a Rust change took about 4 times longer, so the
release profile keeps only `strip = "symbols"`.

## Tried or read about, and left out

- `panic = "abort"`: guard and guard_cursor fail open through `catch_unwind`.
- mimalloc or jemalloc: helps long runs, adds start-up cost and a C build on Windows GNU.
- PGO and BOLT: not on every OS, and `dots` builds on each user's machine.
- A smaller arg parser than clap: `agent-config --help` already starts in about 1.5 ms.
- Calling the launcher by path (its shebang) instead of `sh rust-bin`: 12.2 ms.

## Earlier: the Python to Rust port

`agent-config --help` went from 72.8 ms (Python) to 13.6 ms (Rust), and memory from 15.6 MiB
to 6.7 MiB. That run used another method (a temp repo, 20 runs), so do not compare it with the
tables above.
