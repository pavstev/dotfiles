# Skills

Custom [Claude Code Skills](https://code.claude.com/docs/en/skills) — reusable
instruction packages. Each skill is a folder with a `SKILL.md` (YAML frontmatter +
Markdown), optionally bundled with `scripts/` and reference files.

## How it's wired

This directory is the single source of truth. `../skill-sync.sh` (run by `./dotfiles`,
or by hand) adds a **per-skill symlink** into `~/.claude/skills`. Cursor reads `~/.claude/skills` itself. It never
touches skills it does not own. A skill added here shows up in every agent on every
machine after `dots`.

```
~/dotfiles/claude/skills/ps:foo  ──symlink──▶  ~/.claude/skills/ps:foo
```

Names keep the `ps:` prefix on purpose: it namespaces them like plugin skills and
the slash command stays `/ps:foo`. The open Agent Skills spec forbids the colon, so
these are Claude Code + Cursor skills, not portable ones.

## Rules every skill follows

`check.py` enforces these. It runs from the repo's pre-commit hook whenever a file
under `claude/skills/` is staged, and you can run it any time:

```sh
python3 claude/skills/check.py            # all skills
python3 claude/skills/check.py ps:askme   # one skill
python3 claude/skills/check.py --self-test
```

- `name` equals the folder name. Lowercase, digits, hyphens, optional `ps:` prefix.
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
cp -r _template ps:my-skill
$EDITOR ps:my-skill/SKILL.md      # set name + description, keep the body short
python3 check.py ps:my-skill
dots                              # links it; commit + push to sync machines
```

Test it the way `superpowers:writing-skills` says: run a fresh subagent on a
realistic task without the change, then with it. Fix what the first run got wrong.

## Provenance

See [`SOURCES.md`](SOURCES.md) for where the ideas behind each skill came from.
