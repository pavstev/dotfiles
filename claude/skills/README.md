# Skills

Custom [Claude Code Skills](https://code.claude.com/docs/en/skills) — reusable
instruction packages. Each skill is a folder with a `SKILL.md` (YAML frontmatter +
Markdown), optionally bundled with `scripts/` and reference files.

## How it's wired

This directory is the single source of truth. `dots` (the skills step, `rust/agent-config/src/dots/skills.rs`)
adds a **per-skill symlink** into `~/.claude/skills`. Cursor and Opencode read
`~/.claude/skills` themselves. It never touches skills it does not own. A skill added here
shows up in every agent on every machine after `dots`.

```
~/dotfiles/claude/skills/ps_foo  ──symlink──▶  ~/.claude/skills/ps_foo
```

The `name:` keeps the `ps:` prefix on purpose: it namespaces the skill like a plugin
skill and the slash command stays `/ps:foo`. The folder says `ps_foo`, because Windows
cannot hold a colon in a file name and `git clone` fails there on one (all the rules for
tracked paths: [AGENTS.md](../../AGENTS.md#windows)). The open Agent Skills spec forbids the colon in `name:`, so these are
Claude Code + Cursor + Opencode skills, not portable ones.

## Rules every skill follows

The `skill_check` Rust bin enforces most of them. It runs from the repo's pre-commit hook whenever a file
under `claude/skills/` is staged, and you can run it any time:

```sh
bash claude/hooks/rust-bin skill_check            # all skills
bash claude/hooks/rust-bin skill_check ps:askme   # one skill
cargo test --test runtime_bins                    # its own tests (in rust/agent-config)
```

`skill_check` names each failing rule: the `name` and folder match (`ps:foo` lives in `ps_foo`),
the description (third person, a quoted `: `, length), the body size, hardcoded skill paths, bad
links and junk files in the folder. What it does not check:

- `description` says what the skill does **and when** to use it. It never summarises the workflow:
  an agent that reads a summary skips the body.
- Long material goes in a sibling file linked from `SKILL.md`, one level deep, so the body stays
  under ~5k tokens.
- Bundled scripts run through an interpreter via `${CLAUDE_SKILL_DIR}`. Pre-approve them with
  `allowed-tools: Bash(${CLAUDE_SKILL_DIR}/scripts/*)`. `allowed-tools` only skips prompts; it
  does not fence the agent in.

## Add your own

```sh
cp -r _template ps_my-skill
$EDITOR ps_my-skill/SKILL.md      # set name + description, keep the body short
bash ../hooks/rust-bin skill_check ps:my-skill
dots                              # links it; commit + push to sync machines
```

Test it with fresh subagents: run one on a realistic task without the change,
then one with it. Fix what the first run got wrong.

## A `verify` skill in a project

Claude Code 2.1.286 and later tells Claude to run a skill named `verify` right before each
commit, except for docs-only and test-only changes ([docs](https://code.claude.com/docs/en/skills),
"Run your checks before each commit"). Three things must hold when the session starts: the skill is
a user, project, enterprise or additional-directory skill (or a `.claude/commands/verify.md`), Claude
may invoke it (no `disable-model-invocation: true`), and `includeGitInstructions` is not off. Plugin
skills and the bundled `/verify` do not count.

A project adds it as `.claude/skills/verify/SKILL.md`. Copy the project's fast check from
`rust-bin proj detect` (`check_fast`):

```markdown
---
name: verify
description: "Runs the fast check before a commit. Use before every commit that changes code."
allowed-tools: "Bash(pnpm run check:fast)"
---

Run `pnpm run check:fast` once. If it fails, fix the cause and run it again. Do not commit on a failure.
```

Replace the command with the project's own `check_fast`. The dotfiles repo itself has no `verify`
skill: its checks run in the pre-commit and pre-push hooks.

## Provenance

See [`SOURCES.md`](SOURCES.md) for where the ideas behind each skill came from.
