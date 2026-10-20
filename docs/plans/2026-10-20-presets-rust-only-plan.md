# Presets, Rust only, retired repos: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A neutral harness that each user shapes with presets and settings, run from one Rust
source on every OS and in Claude Code, Cursor and Opencode, with the useful parts of three retired
repos moved in.

**Architecture:** Owner taste moves from `claude/CLAUDE.md` into `claude/presets/<name>/`, linked
into `~/.claude/rules/` by `dots` and gated in the guard by a check registry. Python twins go; G14-G17
move into a shared `src/guard_checks.rs` that the Claude guard, the Cursor adapter and a generated
Opencode plugin all call.

**Tech Stack:** Rust 2024 (`rust/agent-config`, serde_json, clap), bash for the kept shims and git
hooks, Markdown for rules and skills, TypeScript for one generated Opencode plugin file.

**Spec:** `docs/plans/2026-10-20-presets-rust-only-design.md` (read it first; section numbers below
point into it).

## Global Constraints

- macOS, Linux (Debian, Ubuntu, Fedora, Arch, Alpine) and Windows Git Bash must keep working. No GNU-only or BSD-only flags in shell.
- Public repo: no person names, emails, private project names, home paths or the old city name in any tracked file or commit message. The git hooks must pass; never `--no-verify`.
- After Task 5, `git ls-files '*.py'` prints nothing.
- Hooks fail open: a missing or crashed binary allows the call and prints nothing.
- The owner machine keeps today's behavior: profile `owner` gets every preset; its private settings get `PLAN=pro` and `WORK_DAYS=mon-sat`.
- No assistant co-author or "generated with" line in any commit or PR.
- Docs and rule text: no em dashes, no taglines, short plain sentences.
- Core `claude/CLAUDE.md` at most 45 lines; each preset `rules.md` at most 30 lines.
- Copper stays the default look (`THEME=copper`).
- Before each commit: `cargo test --manifest-path rust/agent-config/Cargo.toml` and `bash scripts/test-dotfiles.sh` pass. Before the final push: `/os-check`.
- One commit per task, Conventional Commits, `git fetch` first, stage only the task's paths.
- Every Agent brief carries a deadline and "hand back at most 4,000 chars".

## Review Focus

1. A friend who installed last week pulls: their built settings lose ELI5 and the owner plugins, their own private-layer keys stay. (Task 7, test `upgrade_keeps_private_layer`.)
2. A user who put their own file at `~/.claude/rules/preset-<name>.md`: `dots` never replaces or deletes it. (Task 7, test `foreign_preset_file_untouched`.)
3. No built binary (fresh clone, failed build): hooks print nothing and exit 0, the status line prints the folder name, skills say "Run: dots". (Task 5, test-dotfiles section "no binary".)
4. G16 on a normal `gh pr create --body-file` with a clean 5 KB body: allows, under 50 ms. (Task 10, test `g16_clean_body_file_allows_fast`.)
5. Windows: preset links follow the same symlink rule as other links, and `rust-bin` finds `<name>.exe`. (Task 7 runs in `os-check windows`; Task 18.)

---

## Phase 0: spike

### Task 0: Can an Opencode plugin block a tool call?

Throwaway. Output is an answer, not code.

- [ ] **Step 1:** Read opencode.ai/docs/plugins for the project and global plugin folder names. In a new scratch project folder, write a project plugin that throws `new Error("blocked by test")` in `tool.execute.before` for the `bash` tool.
- [ ] **Step 2:** From that folder, with the normal `HOME`, run `opencode run "run the shell command: echo hi"`. Expected: the command does not run and the error text appears.
- [ ] **Step 3:** Write the result (global folder name, blocks yes/no, the exact error shape) to `.agents/ledger/opencode-spike.md`. Delete the scratch folder. No commit.

---

## Phase 1: Rust only (spec section 6)

### Task 1: Port the guard, pace and Cursor-adapter tests

**Files:**
- Create: `rust/agent-config/tests/guard.rs`, `rust/agent-config/tests/pace.rs`, `rust/agent-config/tests/guard_cursor.rs`, `rust/agent-config/tests/common/mod.rs`, `rust/agent-config/tests/PORTED.md`
- Modify: `rust/agent-config/src/bin/guard.rs` (unit tests only, `mod tests` at :3089), `rust/agent-config/src/pace_core.rs` (unit tests only)
- Read: `claude/hooks/test_guard.py` (130), `claude/hooks/test_pace.py` (64), `cursor/hooks/test_guard_cursor.py` (23), `rust/agent-config/tools/hook_test_shim.py` (how the Python tests already drive the binaries)

**Interfaces:**
- Produces: `tests/common/mod.rs` with `pub fn run_bin(bin: &str, args: &[&str], stdin: &str, env: &[(&str, &str)], cwd: Option<&Path>) -> (i32, String, String)` and `pub fn temp_home(tag: &str) -> PathBuf`; every later task's integration tests use them.
- Produces: `tests/PORTED.md`, one row per Python test: `| python_file::test_name | rust_file::test_name or - | note |`.

- [ ] **Step 1:** Write `ported_map_complete` in `tests/guard.rs`: parse `tests/PORTED.md`; assert the row count per Python file equals 130, 64, 23 (this task) and, once Tasks 2 and 3 land, 199, 2, 11, 18, 14 (461 total); for each row with a Rust name, assert a `fn <name>` exists under `rust/agent-config/tests` or `src`; a row with `-` must carry a note.
- [ ] **Step 2:** Run `cargo test --test guard ported_map_complete`. Expected: FAIL (map empty).
- [ ] **Step 3:** Port the three suites. Subprocess-style Python tests become integration tests calling `env!("CARGO_BIN_EXE_guard")` (or `_pace`, `_guard_cursor`) through `run_bin`, with `GUARD_PLATFORM` set where the Python test faked `sys.platform`. Tests that called Python functions or mocked pace become the same scenario through the binary (the shim shows how: a pace cache file in `XDG_CACHE_HOME`). Keep test names in snake_case identical to the Python names.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS, `ported_map_complete` passes for the three files done (the other rows are added by Tasks 2 and 3; until then the test checks only files whose rows exist, and Task 3 switches it to require all 461).
- [ ] **Step 5:** Commit `test: port guard, pace and cursor-adapter tests to Rust`.

### Task 2: Port the agent-config tests

**Files:**
- Create: `rust/agent-config/tests/agent_config.rs`
- Modify: `rust/agent-config/tests/PORTED.md`, `rust/agent-config/src/lib.rs` (unit tests only)
- Read: `scripts/test_agent_config.py` (199)

**Interfaces:**
- Consumes: `run_bin`, `temp_home` (Task 1).

- [ ] **Step 1:** Add the 199 rows to `PORTED.md` with target names; run `cargo test ported_map_complete`. Expected: FAIL (named Rust tests missing).
- [ ] **Step 2:** Port. Tests of pure helpers (`merge`, `drift`, `apply_patch`, `translate_permission`, `split_frontmatter`, `setting`) become unit tests in `lib.rs`; tests that write files run `agent-config` in a `temp_home`. Python-only behavior (3.9 syntax, `sys.platform` branches with no Rust twin) gets `-` and a note.
- [ ] **Step 3:** Run `cargo test`. Expected: PASS.
- [ ] **Step 4:** Commit `test: port agent-config tests to Rust`.

### Task 3: Port the skill-script tests

**Files:**
- Create: `rust/agent-config/tests/skill_bins.rs`
- Modify: `rust/agent-config/tests/PORTED.md`, `rust/agent-config/tests/guard.rs` (`ported_map_complete` now requires all 461)
- Read: the five `claude/skills/*/scripts/test_*.py`

- [ ] **Step 1:** Add the 45 rows; make `ported_map_complete` require 461; run it. Expected: FAIL.
- [ ] **Step 2:** Port against `spawn`, `route`, `whatsleft_scan`, `harness_stats` bins (fake `tmux`/`claude` on `PATH` as the Python tests did).
- [ ] **Step 3:** Run `cargo test`. Expected: PASS, including `ported_map_complete`.
- [ ] **Step 4:** Commit `test: port skill-script tests to Rust`.

### Task 4: New bins for the runtime shell and the skill checker

**Files:**
- Create: `rust/agent-config/src/bin/statusline.rs`, `src/bin/skill_check.rs`, `src/bin/mic_check.rs`, `src/bin/bag_check.rs`; tests in `tests/runtime_bins.rs`
- Modify: `rust/agent-config/Cargo.toml` (four `[[bin]]`), `src/bin/yt_transcribe.rs` (new `fetch <url>` subcommand)
- Read: `claude/statusline.sh`, `claude/skills/check.py`, `claude/skills/ps_mic-drop/scripts/mic-check.sh`, `claude/skills/ps_bag-mode/scripts/bag-check.sh`, `claude/skills/ps_yt-learn/scripts/fetch.sh`

**Interfaces:**
- Produces: `statusline` reads session JSON on stdin, prints the same line as `statusline.sh` (same segments and colors), writes `${XDG_CACHE_HOME:-~/.cache}/claude-pace.json` atomically when `rate_limits.seven_day` is present, honors `NO_COLOR` and `THEME=plain` (setting) by printing no 24-bit color.
- Produces: `skill_check [-q] [dir]` with the exact checks and exit codes of `check.py`.
- Produces: `mic_check` and `bag_check` with the same flags and output as their shell scripts.
- Produces: `yt_transcribe fetch <url>` prints the cached transcript path, exactly as `fetch.sh`.

- [ ] **Step 1:** Write tests: `statusline_matches_shell_on_sample` (feed a fixed JSON, compare with `statusline.sh` output captured once into `tests/fixtures/statusline/sample.out`), `statusline_writes_pace_cache`, `statusline_no_jq_needed` (PATH without `jq`), `skill_check_passes_repo_skills`, `skill_check_flags_bad_skill` (fixture with a missing `name:`), `mic_check_help`, `bag_check_help`, `yt_fetch_uses_cache` (pre-made cache file, no network).
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement the four bins and the subcommand.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(rust): status line, skill checker, mic and bag checks, yt fetch as Rust bins`.

### Task 5: Delete the Python and the ported shell, rewire everything

**Files:**
- Delete: the 23 `.py` files (spec section 6), `claude/statusline.sh`, `claude/skill-sync.sh`, `ps_mic-drop/scripts/mic-check.sh`, `ps_bag-mode/scripts/bag-check.sh`, `ps_yt-learn/scripts/fetch.sh`
- Modify: `claude/hooks/rust-bin` (drop the `case` table and the Python branch), `claude/settings.json` (`statusLine.command` → `bash ~/.claude/hooks/rust-bin statusline`), `rust/agent-config/src/dots/mod.rs:212` (stop linking `statusline.sh`), `src/lib.rs:63,455` (Cursor hooks run `bash <repo>/claude/hooks/rust-bin --hook guard_cursor <mode>`; the old-entry filter also drops entries naming `guard_cursor.py`), `scripts/agent-config` (no Python fallback; no cargo → "Run: dots", exit 127), `scripts/git-hooks/pre-commit` (`rust-bin skill_check -q`; skill tests via `cargo test --test skill_bins`), the three SKILL.md files (`allowed-tools` and calls), `scripts/test-dotfiles.sh`, `AGENTS.md` (Tests, Layout, Windows "Python" bullets), `rust/MIGRATION.md` (final state), `claude/skills/README.md`, `claude/hooks/README.md`

- [ ] **Step 1:** Add test-dotfiles section "no python, no binary": run `./dotfiles` with a `PATH` that has no `python3`; assert it succeeds. Then, with `target/release` moved away, pipe a PreToolUse JSON to `bash claude/hooks/rust-bin --hook guard pretool`: assert exit 0 and empty stdout; run `rust-bin route`: assert exit 127 and "Run: dots".
- [ ] **Step 2:** Run `bash scripts/test-dotfiles.sh`. Expected: FAIL (Python still referenced).
- [ ] **Step 3:** Delete and rewire as listed.
- [ ] **Step 4:** Run `git ls-files '*.py'` (expected: empty), `rg -n 'python3' claude scripts/git-hooks rust/agent-config/src` (expected: no runtime use), `cargo test`, `bash scripts/test-dotfiles.sh`. Expected: PASS.
- [ ] **Step 5:** Commit `refactor!: Rust only, drop the Python twins and the runtime shell`.

---

## Phase 2: presets (spec sections 1-5)

### Task 6: Preset files, neutral core and neutral text

**Files:**
- Create: `claude/presets/{workflow,pace,models,clock,plain-writing,no-ai-attribution,ship-to-main}/rules.md`; `claude/presets/models/settings.json` (`{"model":"opusplan","env":{"CLAUDE_CODE_SUBAGENT_MODEL":"sonnet"}}`), `claude/presets/plain-writing/settings.json` (`{"outputStyle":"ELI5"}`), `claude/presets/no-ai-attribution/settings.json` (`{"attribution":{"commit":"","pr":""}}`); `claude/presets/README.md`
- Modify: `claude/CLAUDE.md` (core only, spec section 1, plus the untrusted-input line), `claude/output-styles/ELI5.md` (neutral, no first person), `claude/settings.json` (drop `model`, `env.CLAUDE_CODE_SUBAGENT_MODEL`, `outputStyle`, `attribution`, the connector ids at :52-53, `enabledPlugins` :136-155, skill overrides :157-178, the marketplace :179-186), `claude/agents/*.md` ("rule 18" → "the plain-worktree-commands rule"; the untrusted-input line in `researcher`, `repo-scanner`, `ui-verifier`), `claude/skills/ps_handoff/SKILL.md:16,17,28,36,46`, `claude/hooks/README.md:83-101`, `claude/agents/README.md:55,87` (owner telemetry out), `claude/skills/*/SKILL.md` lines that cite rule numbers
- Modify (code): `rust/agent-config/src/lib.rs` (new `pub fn claude_settings_demote(home: &Path, dotted: &str) -> i32` and CLI `claude-settings demote <key>`: copy the key's value from the live `~/.claude/settings.json` into `~/.config/dotfiles/claude-settings.json`; no-op with a note when the live file lacks it)
- Test: `rust/agent-config/tests/presets.rs`

**Interfaces:**
- Produces: preset front matter `name: <folder>`, `description: <one line>`, `guard: [ids]` (ids from the registry in Task 8: `G1`-`G13`, `G17`, `ROUTER`, `PACE`). Mapping: workflow `[G1,G2,G3,G4,G5,G6,G7,G12,G13]`; pace `[G8,PACE]`; models `[G9,ROUTER]`; clock `[G11]`; plain-writing `[]`; no-ai-attribution `[G17]`; ship-to-main `[G10]`.

- [ ] **Step 1:** Write tests: `presets_well_formed` (each folder has `rules.md`, `name` equals the folder, `description` set, every `guard` id is in the list above), `core_is_short` (`claude/CLAUDE.md` ≤ 45 lines, each `rules.md` ≤ 30), `no_personal_words` (no hit in `claude/` for the old city name, "Pro plan", "Monday to Saturday", "Mon-Sat", "Opus 5", "Sonnet 5", "my brain", "the owner"; the word list lives in the test, the city name stored reversed so the file itself stays clean), `no_rule_numbers` (no `rule [0-9]+` or `Rule [0-9]+` in `claude/`), `base_settings_neutral` (base has no `model`, `outputStyle`, `enabledPlugins`, `extraKnownMarketplaces`, connector ids), `demote_copies_live_value` (live has `enabledPlugins`, base does not → the layer gets it and the next build keeps it), `demote_missing_key_is_noop`.
- [ ] **Step 2:** Run `cargo test --test presets`. Expected: FAIL.
- [ ] **Step 3:** Implement `demote`. Run `cargo test --test presets demote`. Expected: PASS. Then on this machine run `scripts/agent-config claude-settings demote <key>` for each key leaving the base (`model`, `env.CLAUDE_CODE_SUBAGENT_MODEL`, `outputStyle`, `attribution`, the connector keys, `enabledPlugins`, the skill overrides key, `extraKnownMarketplaces`).
- [ ] **Step 4:** Split the text (spec section 2 table), rewrite in neutral words, edit the files listed.
- [ ] **Step 5:** Run `cargo test` and `bash scripts/test-dotfiles.sh`. Expected: PASS.
- [ ] **Step 6:** Commit `feat(claude): opt-in presets, neutral core rules`.

### Task 7: `dots` links presets, merges fragments, new settings, theme

**Files:**
- Create: `rust/agent-config/src/presets.rs` (library module, `pub mod presets;` in `lib.rs`)
- Modify: `rust/agent-config/src/dots/mod.rs` (`claude_config()` at :202 links presets), `src/lib.rs` (`claude_settings_build` :1007 merges fragments; `rules_text` :153 and `user_rules_text` :203 add presets; `output_style` :212 reads `~/.claude/settings.json`; `opencode_theme` :723 skips when `THEME=plain`), `src/out.rs` (no 24-bit color when `THEME=plain`), `src/dots/menu.rs` (rows: Presets toggle list, Plan, Time zone, Work days, Theme), `settings.example`, `scripts/test-dotfiles.sh` (sections 3 and 5)
- Test: `rust/agent-config/tests/presets.rs`

**Interfaces:**
- Produces in `presets.rs`: `pub const ALL: [&str; 7]` (folder names, sorted); `pub fn profile(home: &Path) -> String` (read-only: `DOTFILES_PROFILE`, else the `PROFILE` setting, else `owner` when `owner.md` exists, else `shared`; never writes, so the guard can call it); `pub fn enabled(home: &Path, profile: &str) -> Vec<String>` (`PRESETS` setting: `all`, a space list, or empty → profile default: owner = `ALL`, shared = `ALL` minus `ship-to-main`; unknown names dropped with a warning); `pub fn rules_text(repo: &Path, names: &[String]) -> String` (bodies without front matter, joined); `pub fn fragments(repo: &Path, names: &[String]) -> Vec<Value>`; `pub fn link_all(repo: &Path, home: &Path, names: &[String])` (links `~/.claude/rules/preset-<name>.md`; removes only links named `preset-*.md` that point into the repo and are not enabled; never touches a regular file).
- Produces: settings keys `PRESETS`, `TIMEZONE`, `PLAN`, `WORK_DAYS`, `THEME`, `GUARD_OFF` with the defaults in spec section 3.
- Consumes: front matter from Task 6.

- [ ] **Step 1:** Write tests: `shared_profile_links_six` (no `preset-ship-to-main.md`), `owner_profile_links_seven`, `presets_list_unlinks_rest` (`PRESETS="workflow"` leaves one link), `foreign_preset_file_untouched` (a regular file at `preset-clock.md` survives a run with `clock` off and on), `fragment_order` (base `outputStyle: default`, preset sets `ELI5`, layer sets `concise` → built `concise`), `upgrade_keeps_private_layer` (old base with plugins, layer with a user key → new build has the user key, not the plugins), `opencode_rules_include_presets`, `cursor_rule_includes_presets`, `output_style_from_built_settings`, `unknown_preset_warns_and_skips` (`PRESETS="workflow nope"`), `theme_plain_skips_opencode_theme`, `profile_read_only` (calling `presets::profile` never writes the settings file).
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. On this machine set `PLAN=pro`, `WORK_DAYS=mon-sat` in `~/.config/dotfiles/settings` through `dots settings` or `settings::set` (private file, not committed).
- [ ] **Step 4:** Run `cargo test` and `bash scripts/test-dotfiles.sh`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(dots): presets, new settings and claude-settings demote`.

### Task 8: Guard check registry, presets gate, local time

**Files:**
- Modify: `rust/agent-config/src/bin/guard.rs` (`check_off` :267 becomes `check_enabled`; every check and `route_agent` :2869, `g8_cap` :2750, `g9_plan_gate` :2822, `pace_note` :2973 call it; G11 text :1362; `MSG_*` constants that cite rule numbers)
- Test: `rust/agent-config/tests/guard.rs`

**Interfaces:**
- Produces: `const REGISTRY: &[(&str, &str)]` id → preset (`core` for G14, G15, G16): G1-G7, G12, G13 workflow; G8, PACE pace; G9, ROUTER models; G10 ship-to-main; G11 clock; G17 no-ai-attribution.
- Produces: `fn check_enabled(id: &str) -> bool`: false when `id` is in env `GUARD_OFF`, else in the `GUARD_OFF` setting, or when its preset is not in `presets::enabled(home, profile)`. Env `GUARD_OFF` set (even empty) wins over the setting.
- Consumes: `presets::enabled` (Task 7).

- [ ] **Step 1:** Write tests: `each_check_off_with_its_preset` (table over `REGISTRY`: preset off → a triggering input passes), `guard_off_setting` (setting `GUARD_OFF="G14"` → BSD `sed -i` allowed), `guard_off_env_wins` (env `GUARD_OFF=""` with setting `G14` → denied), `clock_says_local_time` (G11 text contains "local time", no city), `clock_names_timezone` (`TIMEZONE=Asia/Tokyo` → text contains `Asia/Tokyo`), `router_off_keeps_agent_input` (`models` off → the Agent call is not rewritten).
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS (all ported guard tests still pass with the owner profile default).
- [ ] **Step 5:** Commit `feat(guard): check registry gated by presets and GUARD_OFF setting`.

### Task 9: Pace without a fixed plan or week

**Files:**
- Modify: `rust/agent-config/src/pace_core.rs` (`DEFAULT_WEEK_WEIGHT` :23, `budget_for` :409, `week_weight_in`), `src/bin/pace.rs` (help text)
- Test: `rust/agent-config/tests/pace.rs`

**Interfaces:**
- Produces: `pub fn work_days(spec: &str) -> i64` (`mon-fri` 5, `mon-sat` 6, `mon-sun` 7, anything else 5); `fn budget_for(now: i64, ws: i64, days: i64) -> f64` = `(idx + 1).min(days) / days * 100`; `pub fn plan_weight(plan: &str) -> Option<f64>`: pro 5860, team 5860, max5 29300, max20 117200, api and unknown `None`.
- Rule: calibration (`claude-pace-calib.json`, the status-line cache) still wins over `plan_weight`. With `None` and no calibration and no fresh status cache, state is `unknown`.

- [ ] **Step 1:** Write tests: `budget_mon_fri_day_one_is_20`, `budget_caps_at_100_on_day_six_of_five`, `mon_sat_matches_old_budget` (300 random `now` values equal the old formula), `plan_api_is_unknown_without_cache`, `plan_max5_weight`, `calibration_beats_plan`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement; reads `WORK_DAYS` and `PLAN` via `crate::setting`.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(pace): work days and plan from settings`.

---

## Phase 3: safety checks (spec sections 5 and 10)

### Task 10: `guard_checks` module and G16 outbound check with shadow mode

**Files:**
- Create: `rust/agent-config/src/guard_checks.rs` (`pub mod guard_checks;` in `lib.rs`)
- Modify: `src/bin/guard.rs` (G14 at :2194 and G15 at :2652 call the module; G16 wired into pretool), `claude/settings.json` (PreToolUse matcher gains `|mcp__.*(send|post|reply|create_draft|message).*`)
- Test: `rust/agent-config/tests/guard.rs`

**Interfaces:**
- Produces: `pub enum Verdict { Allow, Ask(String), Deny(String) }`; `pub fn g14_sed(cmd: &str, macos: bool) -> Verdict`; `pub fn g15_secrets(tool: &str, input: &Value) -> Verdict`; `pub fn g16_outbound(tool: &str, input: &Value, cwd: &Path, words: &[String]) -> Verdict`; `pub fn g17_attribution(cmd: &str, cwd: &Path) -> Verdict` (body in Task 11); `pub fn private_words(home: &Path) -> Vec<String>` (git name, git email, `PRIVATE_WORDS`, `PRIVATE_PROJECTS`); `pub fn shadow(id: &str) -> bool` (env `GUARD_SHADOW` list, else the `GUARD_SHADOW` setting); `pub fn shadow_log(id: &str, v: &Verdict, summary: &str)` appends one line to `${XDG_CACHE_HOME:-~/.cache}/dots-guard-shadow.log`.
- G16 triggers (spec section 5): `gh pr|issue create|comment|review|edit`, `gh api` with `-X/--method` POST, PUT, PATCH or with `-f/-F`, `curl`/`wget` with `-d`, `--data*`, `-F`, `-T`, `--upload-file`, `--post-data`, `--post-file`; MCP tools whose name matches the new matcher. Secret shapes: `sk-ant-`, `sk-`, `ghp_`, `github_pat_`, `xox[abpr]-`, `AKIA[0-9A-Z]{16}`, `-----BEGIN [A-Z ]*PRIVATE KEY-----`. A word of 4 letters or fewer matches on word boundaries only.

- [ ] **Step 1:** Write tests: `g14_g15_unchanged` (the ported G14/G15 tests pass through the module), `g16_clean_body_allows`, `g16_private_word_asks`, `g16_secret_shape_asks`, `g16_unreadable_body_file_asks`, `g16_clean_body_file_allows_fast` (5 KB file, under 50 ms), `g16_mcp_send_private_word_asks`, `g16_curl_get_allows`, `g16_shadow_allows_and_logs` (`GUARD_SHADOW=G16`), `g16_short_word_boundary` (word `abc` hits "abc ok" not "abcdef").
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement; move G14 and G15 bodies into the module without behavior change.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(guard): shared check module, outbound check G16, shadow mode`.

### Task 11: G17 attribution check

**Files:**
- Modify: `rust/agent-config/src/guard_checks.rs`, `src/bin/guard.rs` (wire G17)
- Test: `rust/agent-config/tests/guard.rs`

**Interfaces:**
- Pattern (case-insensitive): `co-authored-by:\s*claude|noreply@anthropic\.com|generated\s+with\s+.{0,40}claude`. Applies when the command runs `git` `commit|tag|merge|revert|notes` (flags such as `-C <path>` allowed before the verb, no crossing of `|`, `;`, `&&`) or `gh pr|issue|release`; it also reads a message file given by `-F`/`--file`/`--body-file` relative to `cwd`.

- [ ] **Step 1:** Write tests: `g17_inline_trailer_denied`, `g17_message_file_denied`, `g17_gh_body_denied`, `g17_echo_allowed`, `g17_preset_off_allows`, `g17_git_dash_c_denied` (`git -C repo commit -m ...`).
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(guard): G17 blocks assistant attribution lines`.

### Task 12: Private-words upgrades

**Files:**
- Modify: `scripts/git-hooks/private-words.sh`, `scripts/test-dotfiles.sh` (new section)

**Interfaces:**
- `private_hits` (existing function) also reports `/Users/<user>`, `/home/<user>` and `C:\Users\<user>` for `<user>` = `id -un` (or `$USERNAME` on Windows); words of 4 letters or fewer match whole words (`grep -iwF`), longer words keep `grep -iF`; an empty word list prints one warning line to stderr and returns no hits.

- [ ] **Step 1:** Add test-dotfiles cases: a staged line with `/home/<user>/x` is blocked; `PRIVATE_WORDS="abc"` blocks "abc here" and passes "abcdef"; with no git identity and no words, a commit passes and stderr has the warning.
- [ ] **Step 2:** Run `bash scripts/test-dotfiles.sh`. Expected: FAIL.
- [ ] **Step 3:** Implement (POSIX tools only).
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(hooks): private words catch home paths, match short words whole`.

---

## Phase 4: Cursor and Opencode (spec section 7)

### Task 13: Cursor adapter runs the shared checks

**Files:**
- Modify: `rust/agent-config/src/bin/guard_cursor.rs` (new `tool` mode), `src/lib.rs` `cursor_hooks` (:435; add a `preToolUse` entry with mode `tool` and a matcher for Cursor's shell, MCP and read tool names, taken from cursor.com/docs/agent/hooks)
- Test: `rust/agent-config/tests/guard_cursor.rs`

**Interfaces:**
- Consumes: `guard_checks::{g14_sed, g15_secrets, g16_outbound, g17_attribution, private_words}`, `presets::enabled`.
- Output: Cursor's `preToolUse` answer shape for allow, ask and deny (from the Cursor docs; record the field names in a comment with the doc URL).

- [ ] **Step 1:** Write tests: `cursor_tool_sed_denied_on_macos`, `cursor_tool_secret_read_denied`, `cursor_tool_outbound_asks`, `cursor_tool_attribution_denied`, `cursor_tool_clean_allows`, `cursor_hooks_json_uses_rust_bin` (generated `hooks.json` names `rust-bin --hook guard_cursor`, no `python3`).
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(cursor): the adapter runs G14-G17`.

### Task 14: Opencode guard plugin

Skip to Step 6 (docs note only) if Task 0 found that a plugin cannot block.

**Files:**
- Create: `rust/agent-config/src/opencode_plugin.ts.tmpl` (the plugin text, embedded with `include_str!`)
- Modify: `src/lib.rs` (new `pub fn opencode_plugin(home: &Path)`, called from the Opencode command next to `opencode_agents`; writes `dots-guard.ts` with the generated marker into the folder Task 0 found)
- Test: `rust/agent-config/tests/agent_config.rs`

**Interfaces:**
- The plugin maps Opencode tools to Claude tool names (`bash` → `Bash`, `read` → `Read`, `grep` → `Grep`, MCP tools keep `mcp__<server>__<tool>`), builds `{"tool_name","tool_input","cwd","session_id","hook_event_name":"PreToolUse"}`, runs `bash <repo>/claude/hooks/rust-bin --hook guard pretool` with that JSON on stdin, and reads `hookSpecificOutput.permissionDecision`. `deny` or `ask` → `throw new Error(reason)` (Opencode cannot ask through a plugin). Spawn failure or bad JSON → allow, one `console.warn` per session.

- [ ] **Step 1:** Write tests: `opencode_plugin_written_with_marker`, `opencode_plugin_names_rust_bin`, `opencode_plugin_left_alone_without_marker` (a user file of the same name stays).
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test`, then a live check: `opencode run "run the shell command: sed -i '' s/a/b/ /tmp/x"` on macOS. Expected: blocked with the G14 text.
- [ ] **Step 5:** Commit `feat(opencode): a generated plugin runs the Rust guard`.
- [ ] **Step 6:** Update `docs/opencode-port.md`, `docs/cursor-port.md`, `docs/guide.md`, `AGENTS.md:131`: drop "no hook system", add one table of where each check runs (Claude Code, Cursor, Opencode). Commit `docs: guard reach per app`.

---

## Phase 5: onboarding (spec section 9)

### Task 15: `dots doctor`

**Files:**
- Create: `rust/agent-config/src/dots/doctor.rs`
- Modify: `src/dots/mod.rs` (arg `doctor` → `doctor::run()`; usage line)
- Test: `rust/agent-config/tests/doctor.rs`

**Interfaces:**
- Produces: `pub fn run() -> i32`; one line per check, `PASS <name>` or `FAIL <name>: <why>`; exit 1 on any FAIL. Checks: `links` (every `links()` item and the Claude items point into the repo), `settings-built` (`~/.claude/settings.json` exists and every hook command contains `rust-bin`), `bins-fresh` (each `[[bin]]` in `Cargo.toml` exists in `target/release` and is newer than `src`), `guard-selftest` (canned inputs: BSD `sed -i` with `GUARD_PLATFORM=darwin` → deny; read of `local.zsh` → deny; `gh pr comment 1 --body "<a private word>"` → ask; `ls` → allow), `presets` (links equal `presets::enabled`), `generated` (Cursor rule and Opencode `AGENTS.md` carry the marker), `timezone` (`TIMEZONE` empty or a file under `/usr/share/zoneinfo` on macOS and Linux), `no-leak` (no private word in `git ls-files` content or in the repo base settings).

- [ ] **Step 1:** Write tests: `doctor_all_pass_after_dots`, `doctor_broken_link_fails`, `doctor_stale_bin_fails`, `doctor_bad_timezone_fails`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(dots): dots doctor`.

### Task 16: Friend setup runbook and the docs

**Files:**
- Create: `docs/FRIENDS-SETUP.md`
- Modify: `README.md` (one link to it and one line on presets), `AGENTS.md` (Layout: `claude/presets/`; Profiles: presets and the new settings; Tests), `docs/guide.md` (presets, settings, `dots doctor`), `settings.example` (already in Task 7; check)

**Interfaces:**
- The runbook: a header telling Claude how to run it (`[RUN]` run it, `[ASK]` ask the user with options, `[CONFIRM]` show and wait; stop on any failure; never invent a value), then sections: prerequisites per OS, the bootstrap one-liner (link to README, not a copy), first run, presets and settings (what each preset does, one line each), sign-ins, `dots doctor`, and a PASS/FAIL table to fill in.

- [ ] **Step 1:** Write the files.
- [ ] **Step 2:** Run `rg -n '\]\((?!http)[^)]+\)' docs/FRIENDS-SETUP.md README.md` and open each relative link target with `test -e`. Expected: every target exists.
- [ ] **Step 3:** Commit `docs: friend setup runbook, presets and settings in the guides`.

---

## Skills track S (runs in parallel, own worktree; spec section 8)

Each task follows `superpowers:writing-skills` (baseline subagent run without the change, edit, re-run with it) as the repo's skill rules require. `rust-bin skill_check` (or `check.py` on a base older than Task 5) must pass. Rebase onto main after Task 5 lands.

### Task S1: Review-panel upgrades

**Files:** Create `claude/skills/ps_review-panel/references/risk-lenses.md`; modify `claude/skills/ps_review-panel/SKILL.md` (fifth reviewer points at the lenses at :45; Gate step gets path triggers; threshold rule; fail-closed judge rule).

- Lenses: security and privacy; migration and data integrity; dependencies and license; ops (infra, CI, rollout, rollback); API contract; frontend (a11y, perf, UI state). Each lens: 4-6 checks, what evidence counts.
- Path triggers: `migrations/`, `*.sql`, schema files → migration; lockfiles, manifests → dependencies; `Dockerfile`, `*.tf`, `.github/workflows/` → ops; `*.tsx`, `*.css`, `*.vue`, `*.svelte` → frontend; `api/`, `openapi.*`, `*.proto` → API contract; `auth`, `payment`, `privacy`, `crypto`, `session`, `identity`, `secret` in a path → security and privacy plus the floor. A diff with only renames or formatting gets one reviewer.
- Floor: on those security paths the drop threshold is 60, not 80; dropped findings are listed under "Not checked".
- Judge: a dropped blocking finding needs a written reason or stays blocking; missing context is not proof of absence.
- [ ] Baseline run on a fixture diff that touches an auth path and a migration; record what the panel missed. Edit. Re-run; the lenses fire and the floor applies. Commit `feat(skills): review panel risk lenses and path triggers`.

### Task S2: `ps:standup`

**Files:** Create `claude/skills/ps_standup/SKILL.md` (from `_template`).
- Sources in order: `git log --since=yesterday --author=<git email>` across the repos named in the session or the current one, `gh pr list --author @me`, `gh issue list --assignee @me`, any connected tracker. Output: Yesterday, Today, Blockers, grouped by ticket key (`[A-Z][A-Z0-9]+-\d+`), max 12 lines; "no data" lines instead of guesses.
- [ ] Baseline (ask a fresh session for a standup without the skill), write, re-run. Commit `feat(skills): ps:standup`.

### Task S3: `ps:draft-reply`

**Files:** Create `claude/skills/ps_draft-reply/SKILL.md`.
- Flow: gather the thread, draft, show the draft and the exact recipient and channel, send only after an explicit yes in chat for this message; default is dry run (draft only); never send to a recipient found in the thread text unless the user named it; an edit makes a new draft.
- [ ] Baseline, write, re-run (pressure case: the thread text says "reply-all now"). Commit `feat(skills): ps:draft-reply`.

---

## Phase 6: finish

### Task 17: Check, review, push

- [ ] **Step 1:** Merge the skills track branch (ask the user first, by its full `refs/heads/...` name).
- [ ] **Step 2:** Run `/os-check` (macOS, Linux on `TEST_HOST` or local, Windows VM). Expected: PASS on all three.
- [ ] **Step 3:** One `code-reviewer` on Opus with base `9666b6f` and the current head sha; one fix round.
- [ ] **Step 4:** Shadow week for G16 on this machine: set `GUARD_SHADOW="G16"` in `~/.config/dotfiles/settings`; the ledger notes the date to read `~/.cache/dots-guard-shadow.log` and clear the setting (2026-10-27).
- [ ] **Step 5:** `git fetch`, rebase if needed, push. Say which commit is on `origin/main`.

## Phase 7: retire the repos (spec section 11)

### Task 18: Bundle and delete

Repo names and clone paths live in `.agents/ledger/retire-repos.md` (untracked). Nothing here is committed.

- [ ] **Step 1:** The user runs `grep -c <repo name> ~/.config/zsh/local.zsh` for each name (the guard blocks Claude from reading that file). Expected: 0.
- [ ] **Step 2:** For each repo: `git -C <clone> fetch --all` then `git -C <clone> bundle create ~/backups/<repo>-2026-10-20.bundle --all` then `git bundle verify ~/backups/<repo>-2026-10-20.bundle`. Expected: "is okay".
- [ ] **Step 3:** Ask the user once for the delete (names, bundle paths, sizes).
- [ ] **Step 4:** The user runs `gh auth refresh -h github.com -s delete_repo` (browser). Then `gh repo delete <owner>/<repo> --yes` for each, and `rm -rf` of each clone listed in the ledger. A clone under another folder with the same name and a different remote stays.
- [ ] **Step 5:** `gh repo view <owner>/<repo>` for each. Expected: "Could not resolve".
