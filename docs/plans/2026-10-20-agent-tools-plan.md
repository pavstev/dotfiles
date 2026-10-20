# Agent tools (ctx and sessions) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two pure-Rust CLIs in `rust/agent-config`: `ctx` (outline and read code by symbol) and `sessions` (search past Claude Code sessions; Opencode and Cursor later), plus a `session-finder` agent and small harness_stats and statusline changes.

**Architecture:** Library modules (`codeview`, `redact`, `stores`) hold the logic; thin bins (`ctx`, `sessions`) parse args and print. `sessions` keeps a tantivy index in the user cache dir and reads Claude Code JSONL. Opencode and Cursor (SQLite) wait for a pure-Rust reader: `turso` was dropped on 2026-10-20 because it needs C (`simsimd`, `bindgen`) on every target. Everything is tested with fixtures in a temp HOME, like the existing `tests/runtime_bins.rs`.

**Tech Stack:** Rust 2024 (rust-version 1.89), clap 4, serde_json, regex, walkdir, dirs 6, tantivy 0.27 (default features off).

**Spec:** `docs/plans/2026-10-20-agent-tools-design.md`

## Global Constraints

- Pure Rust: no crate in the build may be `cc`, `cmake` or `bindgen`, on `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-gnu`.
- Same output on every OS for the same input. Store paths come from one function per store.
- A failed source or file never crashes a tool: it prints `• <tool>: <what> skipped: <why>` and goes on.
- Every command has an output cap and says when it cut.
- No secret is printed or written: `redact::redact` runs before any output or index write.
- Owner decision 2026-10-20: v1 indexes Claude Code only. `read_opencode` and `read_cursor` are not built; `sessions find --source opencode|cursor` prints `• sessions: <source> skipped: not supported yet` and exits 0. A spike checks the pure-Rust reader `sqlite-core` on copies of the live stores; a pass makes Opencode and Cursor a follow-up task after Task 12.
- Never read the Opencode tables `credential`, `account`, `account_state`, `control_account`, `kv`.
- `ctx read` header: `FILE:START-END  <symbol path>  scan=<lang>  hash=<8 hex>`.
- `sessions find` hit: `date  source  project  session-id  title`, then a snippet of at most 200 chars.
- Refuse files over 2 MB or with a line over 10,000 chars.
- Owner writing rules in messages, docs and comments: short plain sentences, no em dashes.
- No `Co-Authored-By`, noreply or "Generated with" line in any commit.
- Before landing on main: `scripts/os-check.sh` on all three OSes (repo `CLAUDE.md`).

## Review Focus

1. A transcript still being written ends in a half JSON line: `sessions index` skips that line and indexes it on the next run (Task 7).
2. Opencode is writing while we index (WAL, maybe locked): the source is skipped with one line, never a hang over 5 s (Task 6).
3. Two sessions run `sessions find` at once: the second waits for the index lock up to 5 s, then searches the old index and says so (Task 7).
4. `ctx` on a binary or non-UTF-8 file: refused with a note, no panic (Task 4).
5. A 100 MB transcript: indexed by streaming, memory stays under 200 MB (Task 7).

---

### Task 1: Dependencies and the no-C gate (spike)

**Files:**
- Modify: `rust/agent-config/Cargo.toml` (add deps and two `[[bin]]` entries for `ctx` and `sessions`, stub mains)
- Create: `rust/agent-config/src/bin/ctx.rs`, `rust/agent-config/src/bin/sessions.rs` (stubs that print usage)
- Create: `rust/agent-config/tests/deps.rs`

**Interfaces:**
- Produces: crate deps `tantivy = { version = "0.27", default-features = false, features = ["mmap", "lz4-compression", "stopwords", "stemmer"] }` (`stopwords` needs `stemmer`); no `turso`; bins `ctx`, `sessions`.

- [ ] **Step 1: Write the failing test** `no_c_build_on_any_target` in `tests/deps.rs`: for each of the 3 targets, run `cargo tree --target <t> -e normal,build --prefix none --format {p}` in `CARGO_MANIFEST_DIR` and assert no line starts with `cc `, `cmake ` or `bindgen `. Also `stub_bins_exist`: `env!("CARGO_BIN_EXE_ctx")` and `env!("CARGO_BIN_EXE_sessions")` run with `--help` and exit 0.
- [ ] **Step 2: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml --test deps`. Expected: FAIL (bins missing).
- [ ] **Step 3: Add the deps, bins and stubs.** If `no_c_build_on_any_target` fails, find the C crate with `cargo tree -i <crate> --target <t>` and turn off the feature that pulls it. (`turso` was tried and dropped: it needs C. See the notes folder file `task1-turso-needs-c.md`.)
- [x] **Step 4: Probe the live database.** Dropped with `turso`. The `sqlite-core` spike replaces it (separate helper, scratch folder, copies only).
- [ ] **Step 5: Measure build cost.** Time a clean `cargo build --release --bins` twice: in a second git worktree at the parent commit (before) and here (after), each with its own empty `target/`. Record both times in the commit message. If the clean build grew more than 120 s, STOP and report.
- [ ] **Step 6: Run** the deps test again. Expected: PASS.
- [ ] **Step 7: Commit** `feat(agent-tools): tantivy dep with a no-C gate`.

### Task 2: codeview edge-case fixtures and failing tests (Opus helper)

**Files:**
- Create: `rust/agent-config/tests/fixtures/codeview/<case>.<ext>` and `<case>.<ext>.outline` for each case
- Create: `rust/agent-config/tests/codeview.rs`

**Interfaces:**
- Produces (the API Task 3 must build, in `src/codeview.rs`, `pub mod codeview;` in `lib.rs`):
  - `pub struct Symbol { pub kind: String, pub name: String, pub start: usize, pub end: usize, pub depth: usize }` (lines 1-based, inclusive)
  - `pub enum Refusal { TooBig, LongLine, NotText, NoScanner(String) }`
  - `pub fn scan_name(path: &Path) -> Option<&'static str>` (returns `"c-like"`, `"indent"`, `"markdown"`, `"shell"`)
  - `pub fn outline(path: &Path, text: &str) -> Result<Vec<Symbol>, Refusal>`
  - `pub fn render(symbols: &[Symbol]) -> String`: one line per symbol, `"  ".repeat(depth) + "{kind} {name} {start}-{end}"`

- [ ] **Step 1: Write one fixture pair per case.** Cases (from the spec): nested functions (ts); multi-line arrow functions (ts); multi-line signatures and generics with `where` (rs); decorators (ts, py); braces inside strings, template literals with `${}`, regex literals and comments (ts); `impl` blocks, `macro_rules!`, raw strings `r#"..."#` (rs); nested defs, multi-line strings, line continuations (py); `#` inside fenced code (md); shell `name() {` and `function name` (sh); CSS selectors with nested braces in strings (css); empty file (ts); one-line minified file over 10,000 chars (js); CRLF line ends (ts); tabs (py); unknown extension (`.xyz`). The `.outline` file is the exact `render()` output, written by hand from reading the fixture.
- [ ] **Step 2: Write the test** `every_fixture_matches_its_outline`: for each fixture with an `.outline` file, `render(&outline(path, &text).unwrap())` equals the file's text. Plus `refusals`: the minified file gives `Refusal::LongLine`, a 2 MB + 1 byte string gives `Refusal::TooBig`, text with a NUL byte gives `Refusal::NotText`, `.xyz` gives `Refusal::NoScanner("xyz")`.
- [ ] **Step 3: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml --test codeview`. Expected: FAIL to compile (no `codeview` module).
- [ ] **Step 4: Commit** `test(codeview): edge-case fixtures for the line scan`.

### Task 3: codeview line scan

**Files:**
- Create: `rust/agent-config/src/codeview.rs`
- Modify: `rust/agent-config/src/lib.rs` (add `pub mod codeview;`)

**Interfaces:**
- Consumes: the API in Task 2.
- Produces: the same API, passing Task 2's tests.

- [ ] **Step 1: Implement** the four scanners from the spec table. The C-like brace counter is a small state machine over chars with states `Code`, `LineComment`, `BlockComment`, `Str(quote)`, `Template(depth of ${})`, `Regex`, `RawStr(hashes)`; a `/` starts a regex only after `(`, `,`, `=`, `:`, `[`, `!`, `&`, `|`, `?`, `{`, `}`, `;` or line start. A symbol start line is matched with anchored regexes per family; its end is the line where the brace depth returns to the depth before its `{`. Indent family: end is the last non-blank line before a line at the same or lower indent that is not inside a multi-line string or after a `\` continuation. Decorator lines above a start line move `start` up.
- [ ] **Step 2: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml --test codeview`. Expected: PASS, every fixture.
- [ ] **Step 3: Run** `cargo clippy --manifest-path rust/agent-config/Cargo.toml --all-targets -- -D warnings`. Expected: no warnings.
- [ ] **Step 4: Commit** `feat(codeview): line-scan outline for five language families`.

### Task 4: `ctx` bin

**Files:**
- Modify: `rust/agent-config/src/bin/ctx.rs`
- Create: `rust/agent-config/tests/ctx.rs`

**Interfaces:**
- Consumes: `codeview::{outline, render, scan_name, Refusal, Symbol}`.
- Produces: CLI `ctx outline FILE`, `ctx read FILE SYMBOL [--if-changed HASH]`. Exit 0 on output, 1 on refusal or not found, 2 on usage error. Hash = first 8 hex of sha256 of the file bytes (use the existing `sha2` dep).

- [ ] **Step 1: Write failing tests** in `tests/ctx.rs` (use `common::run_bin`):
  - `outline_prints_render`: output equals `render` for `tests/fixtures/codeview/nested.ts`.
  - `read_prints_header_and_numbered_lines`: first line matches `^.+nested.ts:\d+-\d+  outer\.inner  scan=c-like  hash=[0-9a-f]{8}$`; the next lines are `{n}\t{line}`.
  - `read_if_changed_same_hash_says_unchanged`: second call with the printed hash prints exactly `unchanged (hash <h>)`.
  - `read_not_found_lists_closest`: unknown symbol exits 1 and lists at most 5 names.
  - `read_ambiguous_lists_all`: a name defined twice lists both ranges, exit 1.
  - `binary_file_refused`: a file with NUL bytes exits 1 with `not text`, no panic.
  - `output_cap`: a symbol over 400 lines prints the first 400 and a line `… cut at 400 of N lines; use Read with offset`.
- [ ] **Step 2: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml --test ctx`. Expected: FAIL.
- [ ] **Step 3: Implement** `ctx` with clap. Symbol path = names joined by `.` from the outermost symbol to the match. Closest names by edit distance (write a 15-line Levenshtein, no new dep).
- [ ] **Step 4: Run** the ctx tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ctx): outline and read code by symbol`.

### Task 5: redactor

**Files:**
- Create: `rust/agent-config/src/redact.rs`; modify `src/lib.rs` (`pub mod redact;`)
- Test: unit tests inside `redact.rs`

**Interfaces:**
- Produces: `pub fn redact(text: &str) -> std::borrow::Cow<'_, str>`; replaces each hit with `[REDACTED]`.

- [ ] **Step 1: Write failing unit tests** with fake shapes built at runtime (string concat, so the pre-commit scanner does not flag the source): `sk-` + 40 chars, `sk-ant-` + 90 chars, `ghp_` + 36, `github_pat_` + 82, `AKIA` + 16 caps, `xox[bp]-` tokens, `Bearer ` + 30 chars, a `-----BEGIN ... PRIVATE KEY-----` block, and a name holding key/token/secret/password assigned a value of 16+ chars (`=` or `:`). Each must be gone from the output. Plain prose and a 15-char value stay unchanged (`Cow::Borrowed`).
- [ ] **Step 2: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml --lib redact`. Expected: FAIL.
- [ ] **Step 3: Implement** with one `regex::RegexSet` for the fast check and the matching `Regex` list for the replace, built once in a `LazyLock`.
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(redact): one redactor for tool output and indexes`.

### Task 6: store readers (Claude only in v1)

**Files:**
- Create: `rust/agent-config/src/stores/mod.rs`, `claude.rs`, `opencode.rs`, `cursor.rs`; modify `src/lib.rs` (`pub mod stores;`)
- Create: `rust/agent-config/tests/stores.rs`; fixtures built at test time in a temp HOME

**Interfaces:**
- Produces:
  - `pub struct Doc { pub source: &'static str, pub session: String, pub project: String, pub title: String, pub time: i64 /* epoch secs */, pub role: String, pub text: String }`
  - `pub enum Skip { Missing, Locked, Error(String) }`
  - `pub fn claude_root(home: &Path) -> PathBuf`, `pub fn opencode_db(home: &Path) -> PathBuf`, `pub fn cursor_db(home: &Path) -> PathBuf`
  - `pub fn read_claude(home: &Path, since: &BTreeMap<PathBuf,(u64,i64)>, out: &mut dyn FnMut(Doc)) -> Result<BTreeMap<PathBuf,(u64,i64)>, Skip>` (map = path to (size, mtime); unchanged files are skipped)
  - v1 builds `read_claude` only. `read_opencode` and `read_cursor` are not built (follow-up after the `sqlite-core` spike); `opencode_db` and `cursor_db` stay as path functions.
  - Every `Doc.text` has passed `redact::redact`.

- [ ] **Step 1: Write failing tests:**
  - `paths_per_os`: on macOS `cursor_db` ends in `Library/Application Support/Cursor/User/globalStorage/state.vscdb`, on Linux `.config/Cursor/User/globalStorage/state.vscdb`, on Windows `AppData/Roaming/Cursor/User/globalStorage/state.vscdb`; `opencode_db` is `.local/share/opencode/opencode.db` on all three (`cfg!` branches in the test).
  - `claude_reads_main_and_subagents`: two JSONL fixtures (one under `subagents/`) give the expected user, assistant-text and tool docs; tool docs hold the tool name and its `command` or `file_path`, never the tool output.
  - `claude_half_last_line_skipped`: a file ending in a half JSON line yields the docs before it and no error.
  - (Opencode and Cursor fixture and lock tests move to the follow-up task.)
  - `forbidden_tables_never_named`: read the text of `src/stores/*.rs` and assert none of `credential`, `account`, `account_state`, `control_account`, ` kv` appears in a SQL string.
  - `secrets_redacted`: a fake key in a fixture prompt is `[REDACTED]` in the doc.
- [ ] **Step 2: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml --test stores`. Expected: FAIL.
- [ ] **Step 3: Implement.** Claude: stream with `BufReader::lines`, parse each line with serde_json, skip lines that do not parse. Title from the desktop app records (the same lookup `harness_stats` uses; move it to `stores::claude` and call it from both). Opencode and Cursor readers wait for the follow-up.
- [ ] **Step 4: Run** the stores tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(stores): read Claude, Opencode and Cursor sessions`.

### Task 7: `sessions` bin

**Files:**
- Modify: `rust/agent-config/src/bin/sessions.rs`
- Create: `rust/agent-config/tests/sessions.rs`

**Interfaces:**
- Consumes: `stores::*`, `redact::redact`.
- Produces: CLI `sessions index [--rebuild]`, `sessions find QUERY [--project P] [--since YYYY-MM-DD] [--source claude|opencode|cursor] [--limit N]` (default limit 10, max 50). Index dir `dirs::cache_dir()/dots/sessions/`, state file `state.json` there with keys `claude` (path map) and `version` (1).

- [ ] **Step 1: Write failing tests** (temp HOME, fixtures from Task 6 helpers):
  - `find_returns_hit_block`: after indexing fixtures, `find "flaky vitest"` prints a line matching `^\d{4}-\d{2}-\d{2}  claude  \S+  [0-9a-f-]{36}  .+$` and a snippet of at most 200 chars.
  - `incremental_index`: a second `index` with no file changes adds 0 docs (print `• sessions: 0 new`); appending one JSONL line adds 1.
  - `half_line_indexed_next_run`: a half last line is skipped, then indexed after it is completed.
  - `unsupported_source_one_line`: `find x --source opencode` (and `cursor`) prints `• sessions: opencode skipped: not supported yet` and exits 0.
  - `lock_wait_then_stale`: hold the tantivy writer lock in the test; `find` returns within 6 s, with the line `• sessions: index busy, searched the last index`.
  - `filters`: `--source`, `--project`, `--since` and `--limit` each narrow the hits.
  - `no_secret_in_index`: grep the index dir bytes for the fake key text: not found.
  - `big_transcript_streams`: a generated 100 MB JSONL indexes with peak RSS under 200 MB (read RSS with `/usr/bin/time -l` on macOS, `/proc/self/status` on Linux; skip on Windows with a note).
- [ ] **Step 2: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml --test sessions`. Expected: FAIL.
- [ ] **Step 3: Implement.** Schema: `source` and `session` and `project` as STRING|STORED, `title` and `text` as TEXT|STORED, `time` as i64 INDEXED|STORED|FAST, `role` as STRING|STORED. Writer heap 50 MB. Commit once per run. Snippet with tantivy's `SnippetGenerator`, cut to 200 chars.
- [ ] **Step 4: Run** the sessions tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(sessions): search past sessions across three agent apps`.

### Task 8: `session-finder` agent and `ps:sessions`

**Files:**
- Create: `claude/agents/session-finder.md`
- Modify: `claude/skills/ps_sessions/SKILL.md` (one line: use `session-finder` or `rust-bin sessions find` before grepping transcripts), `claude/agents/README.md` (one row), `claude/presets/workflow/rules.md` (add `session-finder` to the Agents line)

**Interfaces:**
- Consumes: `~/.claude/hooks/rust-bin sessions find`.

- [ ] **Step 1: Baseline (skill TDD).** Headless `claude -p` in a temp folder: "Find the session where we set up the X1 Beszel dashboard." Record tool calls and whether it greps `~/.claude/projects`.
- [ ] **Step 2: Write the agent:** frontmatter `name: session-finder`, `model: haiku`, `tools: Bash, Read`; body: run `rust-bin sessions find`, open at most 2 transcripts, hand back at most 1,500 chars with session ids, dates and titles; never print secrets.
- [ ] **Step 3: Edit** `ps:sessions`, the README row and the workflow Agents line.
- [ ] **Step 4: Re-run** the same headless test in a new session. Expected: it calls `sessions find` or the agent; fewer tool calls than the baseline. Save both runs in `.agents/tdd/2026-10-20-agent-tools/`.
- [ ] **Step 5: Run** `~/.claude/hooks/rust-bin skill_check` and `cargo test --manifest-path rust/agent-config/Cargo.toml --test skill_bins`. Expected: PASS.
- [ ] **Step 6: Commit** `feat(agents): session-finder on the sessions index`.

### Task 9: harness_stats changes

**Files:**
- Modify: `rust/agent-config/src/bin/harness_stats.rs`
- Test: add to the existing harness_stats tests (`rg -l harness_stats rust/agent-config/tests`)

**Interfaces:**
- Consumes: the moved title lookup from Task 6 (no Opencode or Cursor source in v1).

- [ ] **Step 1: Write failing tests:** `collect_drops_headless_tests` (a project folder starting `-private-` or under the temp dir, with no app record and at most 3 calls, is dropped; `--all` keeps it); `report_has_new_rows` (rows `questions from ask skills` and `Stop blocks per check` exist; the row `agent calls w/ effort %` is gone); the Opencode row test waits for the follow-up.
- [ ] **Step 2: Run** them. Expected: FAIL.
- [ ] **Step 3: Implement.** "Questions from ask skills" = AskUserQuestion calls in sessions whose skills include a name matching `askme|brainstorm|grilling|design-brainstorm|pick`. "Stop blocks per check" = count of `Stop hook feedback` records per `G\d+` code.
- [ ] **Step 4: Run** the tests. Expected: PASS. Update `claude/skills/ps_harness-optimize/SKILL.md` step 3 metric list (skill TDD: one headless run before and after).
- [ ] **Step 5: Commit** `feat(harness_stats): drop test runs, ask-skill and Stop rows`.

### Task 10: statusline "what is left"

Start only after the guard quick-wins branch (G10 state) is on main: both edit the guard's Stop pass.

**Files:**
- Modify: `rust/agent-config/src/bin/guard.rs` (Stop pass), `rust/agent-config/src/bin/statusline.rs`, `rust/agent-config/src/lib.rs`
- Test: `rust/agent-config/tests/guard.rs`, `rust/agent-config/tests/runtime_bins.rs`

**Interfaces:**
- Produces: `pub fn status_path(session: &str) -> PathBuf` in `lib.rs` = `dirs::cache_dir()/dots/status/<session>.txt`; file holds one line `<n>\t<first open step>`, or is removed when nothing is open.

- [ ] **Step 1: Write failing tests:** guard Stop on a transcript with 2 open plan steps writes `2\tRun tests`; with none, the file is gone. Statusline with that file and the matching `session_id` in its input ends with `▸ 2 left: Run tests`; a step over 40 chars is cut with `…`; no file, no change to today's output (the existing fixtures stay byte-identical).
- [ ] **Step 2: Run** them. Expected: FAIL.
- [ ] **Step 3: Implement** using the open steps the Stop pass already reads for G12 (`Turn.plan`).
- [ ] **Step 4: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml`. Expected: all PASS.
- [ ] **Step 5: Commit** `feat(statusline): show what is left in the session`.

### Task 11: guard hint, rule line and docs

**Files:**
- Modify: `rust/agent-config/src/bin/guard.rs` (new hint, not a deny), `rust/agent-config/tests/guard.rs`
- Modify: `claude/presets/workflow/rules.md` (one line naming `ctx`), `AGENTS.md` (layout rows for `ctx` and `sessions`), `docs/ai-agents.md` (where the guard checks table lists hints)

**Interfaces:**
- Produces: guard PreToolUse additional context `Tip: ctx outline <file>, then ctx read <file> <symbol>` when Bash runs `sed -n` or `cat` on a file `codeview::scan_name` knows; at most 3 per session (counter in the existing per-session state).

- [ ] **Step 1: Write failing tests:** `sed -n '1,80p' src/a.ts` gets the hint and is allowed; `cat notes.xyz` gets nothing; the 4th hint in one session is silent; `GUARD_OFF` with the new check code turns it off.
- [ ] **Step 2: Run** them. Expected: FAIL.
- [ ] **Step 3: Implement** the hint and the doc lines. Name the check `H1` and add it to the `GUARD_OFF` doc comment.
- [ ] **Step 4: Run** `cargo test --manifest-path rust/agent-config/Cargo.toml` and clippy. Expected: PASS, no warnings.
- [ ] **Step 5: Commit** `feat(guard): hint ctx when code is read through the shell`.

### Task 12: Check on three OSes and land

- [ ] **Step 1: Run** `bash scripts/test-dotfiles.sh`. Expected: last line says done, exit 0.
- [ ] **Step 2: Run** `bash scripts/os-check.sh --since origin/main`. Expected: macOS, Linux and Windows PASS. On UTM error -609 (exit 142), quit and reopen UTM and run again.
- [ ] **Step 3: Final review** with `code-reviewer` (Opus, high) on the branch from `origin/main` to HEAD.
- [ ] **Step 4: Land** on main per the ship-to-main preset, then run `dots` and check `~/.claude/hooks/rust-bin sessions find "Beszel"` returns a hit.
- [ ] **Step 5:** Follow-up task (after landing): Opencode and Cursor readers on `sqlite-core`, if the spike passes. Spike result: notes folder.
- [ ] **Step 6:** Add a line to `~/.claude/research/2026-10-20-harness-research.md`: measure on 2026-10-27 together with the compact trial.
