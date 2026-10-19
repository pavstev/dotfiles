# Skills

Custom [Claude Code Skills](https://code.claude.com/docs/en/skills) — reusable
instruction packages. Each skill is a folder with a `SKILL.md` (YAML frontmatter +
Markdown), optionally bundled with `scripts/` and reference files.

## How it's wired

This directory is the single source of truth. `../skill-sync.sh` (run by `./dotfiles`,
or by hand) adds a **per-skill symlink** into `~/.claude/skills`. Cursor and Opencode read
`~/.claude/skills` themselves. It never touches skills it does not own. A skill added here
shows up in every agent on every machine after `dots`.

```
~/dotfiles/claude/skills/ps_foo  ──symlink──▶  ~/.claude/skills/ps_foo
```

The `name:` keeps the `ps:` prefix on purpose: it namespaces the skill like a plugin
skill and the slash command stays `/ps:foo`. The folder says `ps_foo`, because Windows
cannot hold a colon in a file name and `git clone` fails there on one. No tracked path
may hold a character Windows rejects (`scripts/test-dotfiles.sh` and the pre-commit hook
check this). The open Agent Skills spec forbids the colon in `name:`, so these are
Claude Code + Cursor + Opencode skills, not portable ones.

## Rules every skill follows

`check.py` enforces these. It runs from the repo's pre-commit hook whenever a file
under `claude/skills/` is staged, and you can run it any time:

```sh
python3 claude/skills/check.py            # all skills
python3 claude/skills/check.py ps:askme   # one skill
python3 claude/skills/check.py --self-test
```

- `name` equals the folder name with `:` written as `_` (`ps:foo` lives in `ps_foo`). Lowercase, digits, hyphens, optional `ps:` prefix.
- `description` is third person, says what the skill does **and when** to use it, and
  is quoted whenever it contains `: ` (an unquoted colon silently drops the skill).
  It never summarises the workflow — an agent that reads a summary skips the body.
- Body under 500 lines and ~5k tokens. Long material goes in a sibling file linked
  from `SKILL.md`, one level deep.
- Bundled scripts are run through an interpreter via `${CLAUDE_SKILL_DIR}`, never a
  hardcoded `~/.claude/skills/...` path. Pre-approve them with
  `allowed-tools: Bash(${CLAUDE_SKILL_DIR}/scripts/*)` so they run without a prompt.
- `allowed-tools` is scoped (`Bash(go *)`), never bare `Bash`. It only skips prompts;
  it does not fence the agent in.
- No tool caches or `.DS_Store` inside a skill folder.

## Add your own

```sh
cp -r _template ps_my-skill
$EDITOR ps_my-skill/SKILL.md      # set name + description, keep the body short
python3 check.py ps:my-skill
dots                              # links it; commit + push to sync machines
```

Test it the way `superpowers:writing-skills` says: run a fresh subagent on a
realistic task without the change, then with it. Fix what the first run got wrong.

## Provenance

See [`SOURCES.md`](SOURCES.md) for where the ideas behind each skill came from.
