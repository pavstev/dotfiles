# Agent tools in Rust: ctx and sessions (design, 2026-10-20)

Status: design, approved in the brainstorm of 2026-10-20. Waits for the owner's review of this file.
Evidence: `~/.claude/research/2026-10-20-harness-research.md` (sections 1.5, 3, 5).

## Goal

Cut the tokens and turns agents spend reading code and looking for past work, with two pure-Rust
tools that behave the same on macOS, Linux and Windows.

## Why

From 157K tool calls (Oct 1 to 20, 2026):
- Agents read code through the shell: 18,611 `sed -n` slices (21.6M tokens), 11,588 `cat FILE`
  (19.3M tokens), 3,332 reads of a file already read 3 or more times (8.3M tokens).
- About 1,700 transcript reads went to "where did we do X". The built-in transcript search was
  used 5 times.

## Decisions (from the brainstorm)

| # | Decision | Rejected |
|---|---|---|
| 1 | Both tools live in `rust/agent-config`, as new bins | a new repo |
| 2 | `ctx` is a CLI first | an MCP server first (later, it can wrap the same code) |
| 3 | `ctx` uses a line scan on every OS; no tree-sitter | tree-sitter (needs a C compiler; Windows GNU host has none) |
| 4 | An Opus helper writes the line-scan edge-case tests first (TDD) | |
| 5 | `sessions` indexes Claude Code, Opencode and Cursor | Claude only |
| 6 | SQLite is read with the pure-Rust `turso` crate (0.8.x), read-only | the `sqlite3` command, a C compiler |
| 7 | A Haiku `session-finder` agent answers "where did we do X" | CLI only |
| 8 | `ctx test` (test-impact) is parked | |

Accepted risks: a line-scan cut can be wrong; `turso` is pre-1.0 and reads a live database.

## Rules for every part

- Pure Rust. No crate may build C code (`cc`, `cmake`, `bindgen`). A test in `tests/` checks
  `cargo tree` for those crates.
- Same output on every OS for the same input. Paths come from one function per store, with a test
  per OS.
- A failed source or file never crashes a tool. It prints one line `• <tool>: <what> skipped: <why>`
  and goes on.
- Output stays short. Every command has a cap and says when it cut.
- No secret is printed or written. One new redactor module (`src/redact.rs`) runs before any output
  or index write. The guard has no redactor today.

## Part 1: `ctx`

### Commands

```text
ctx outline FILE              symbols with line ranges, nested by indent
ctx read FILE SYMBOL          one symbol's lines, numbered like the Read tool
ctx read FILE SYMBOL --if-changed HASH   prints "unchanged" when the file hash matches
```

`ctx read` starts with one header line: `FILE:START-END  <symbol path>  scan=<lang>  hash=<8 hex>`.
The agent sees the range the scan used, so a bad cut is visible.

### Line scan (module `src/codeview.rs`)

One scanner per language family, all in pure Rust, no regex backtracking on whole files:

| Family | Extensions | Symbol starts | Symbol end |
|---|---|---|---|
| C-like | ts, tsx, js, jsx, mjs, cjs, rs, go, java, css, scss | `function`, `class`, `interface`, `type`, `enum`, `const x = (...) =>`, `export ...`, `fn`, `impl`, `struct`, `trait`, `mod`, a CSS selector line | matching brace, counted while skipping strings, template literals, regex literals, char literals and comments |
| Indent | py | `def`, `async def`, `class`, with decorators above | next line at the same or lower indent that is not blank, a comment or a continuation |
| Markdown | md, mdx | `#` headings outside fenced code | next heading of the same or higher level |
| Shell | sh, bash, zsh | `name() {`, `function name` | matching brace |
| Other | anything else | none | `ctx outline` prints the line count and says "no scanner"; `ctx read` refuses with that note |

Limits: files over 2 MB or with a line over 10,000 chars are refused with a note (minified code).
CRLF and tabs are handled. A symbol name that matches more than once lists all matches with ranges.
A symbol that is not found lists the 5 closest names.

### Edge-case tests (written first, by an Opus helper)

Fixtures in `tests/fixtures/codeview/`, one file per case, each with the expected outline:
nested functions; multi-line arrow functions; multi-line signatures and generics with `where`;
decorators (TS, Python); braces inside strings, template literals with `${}`, regex literals and
comments; Rust `impl` blocks, `macro_rules!`, raw strings `r#"..."#`; Python nested defs,
multi-line strings and line continuations; Markdown with `#` inside fenced code; an empty file; a
one-line minified file; CRLF; tabs; an unknown extension.

### Nudge

- A new guard hint (not a deny): when a Bash call runs `sed -n` or `cat` on a file with a known
  extension, the hook adds one line of context: `Tip: ctx outline <file>, then ctx read <file> <symbol>`.
  At most 3 hints per session.
- One line in `claude/presets/workflow/rules.md` names `ctx`.

### Not in v1

An automatic "unchanged since your last read" reply. Subagents share `CLAUDE_CODE_SESSION_ID` with
the main thread, so a shared cache could tell a helper "unchanged" for a file it never read.
`--if-changed HASH` gives the same saving without that risk.

## Part 2: `sessions`

### Commands

```text
sessions find QUERY [--project P] [--since DATE] [--source claude|opencode|cursor] [--limit 10]
sessions index [--rebuild]
```

`sessions find` updates the index first (incremental), then prints one block per hit:
`date  source  project  session-id  title` and a snippet of at most 200 chars around the match.

### Index

- tantivy (pure Rust, 0.27), in `<cache dir>/dots/sessions/` (`dirs::cache_dir()`).
- One document per message: source, session id, project, title, time, role, text. Text = user
  prompt, assistant text, tool name plus its command or path, tool error text. Tool output bodies are
  not indexed.
- Incremental state in `state.json`: Claude by file path, size and mtime; Opencode by the highest
  `part.time_updated` seen; Cursor is re-read in full (it is small).

### Sources

| Source | Where | Read |
|---|---|---|
| Claude Code | `~/.claude/projects/*/*.jsonl` and `*/subagents/*.jsonl`; app records for title | streamed line by line |
| Opencode | `opencode.db` in the Opencode data dir | `turso`, read-only; tables `session`, `message`, `part` only |
| Cursor | `state.vscdb` in Cursor's `User/globalStorage` | `turso`, read-only; `cursorDiskKV` rows `composerData:*` and `bubbleId:*` only |

Never read the Opencode tables `credential`, `account`, `account_state`, `control_account` or `kv`.
A test fails if the code names them.

Opencode runs in WAL mode. The reader opens read-only and never writes or checkpoints. If `turso`
cannot read the database, the source is skipped with one line.

### Redaction

Before a document is written: `src/redact.rs` replaces API keys, tokens, `Bearer` values,
private key blocks and long values assigned to names that hold key, token, secret or password, with
`[REDACTED]`.
Tests feed known fake key shapes and check the index holds none.

## Part 3: `session-finder` agent

`claude/agents/session-finder.md`, model Haiku, tools Bash and Read. It runs
`rust-bin sessions find`, reads at most 2 transcripts for detail, and hands back at most 1,500 chars
with session ids and dates. `ps:sessions` points to it (skill TDD).

## Part 4: harness_stats and statusline (small)

- `harness_stats collect` drops headless test runs (a temp-dir project with no app record and at most
  3 calls); `--all` keeps them.
- `report` replaces the "agent calls w/ effort %" row with "questions from ask skills" and adds
  "Stop blocks per check".
- `harness_stats` reads Opencode and Cursor through the same store readers as `sessions`.
- The guard's Stop pass writes the open task steps to `<cache dir>/dots/status/<session>.txt`; the
  statusline shows `▸ 2 left: <first open step>`, cut to 40 chars.

## Plan order

1. Spike: build `turso` for `x86_64-pc-windows-gnu` with no C toolchain, and read the live
   `opencode.db` read-only. If either fails, stop and ask the owner.
2. Opus helper writes the codeview fixtures and failing tests.
3. `codeview.rs` and the `ctx` bin until the tests pass.
4. Store readers, redactor, `sessions` bin and its tests.
5. `session-finder` agent; `ps:sessions` edit (skill TDD).
6. harness_stats and statusline changes.
7. Guard hint and the workflow rule line.
8. `scripts/os-check.sh` on all three OSes, then land on main.

## Measure after one week

With `harness_stats` and a sweep script: `sed -n` and `cat` reads per 100 calls, repeat reads,
`ctx` calls, transcript reads, `sessions find` calls, and `/ps:whatsleft` asks. Compare with the
2026-10-20 numbers above.

## Cost

- Build: about 3 to 4 helper days.
- Clean build time of the crate grows (tantivy, turso). Measure it in step 1; if the first `dots`
  run on a new machine gets more than 2 minutes slower, ask the owner before step 4.
- Start tokens: 0 for the CLIs; one agent description for `session-finder`.
