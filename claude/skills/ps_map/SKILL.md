---
name: ps:map
description: "Writes or refreshes a short folder map (MAP.md) for a repo and adds one line to its root CLAUDE.md to read the map first. Use when the user asks for a repo map, a folder map or a README map, or wants agents to find files faster in a project."
argument-hint: "[repo path — empty = the current repo]"
allowed-tools: "Bash(bash ${CLAUDE_SKILL_DIR}/scripts/map-tree.sh *)"
---

# Map

Input: $ARGUMENTS

Write `MAP.md` at the repo root: where things live, so an agent reads one short file instead of searching.
Every path in it must exist. The map stays under 60 lines.

## Steps

1. The repo is the path in the input, else the current repo. Run `bash ${CLAUDE_SKILL_DIR}/scripts/map-tree.sh tree <repo>`. It lists the tracked folders (two levels, three under a folder of 30 or more files), file counts, root files and script names. For a deeper look use `git ls-files <folder>`. Use only these paths, never ones from memory.
2. Read the README, the root `CLAUDE.md` or `AGENTS.md` and the manifest. For a folder whose use the name does not tell, read the top of one file. Do not copy what those files already say: link them.
3. Write `MAP.md` in the shape below. If `MAP.md` exists and its first line is `<!-- ps:map -->`, refresh it: keep rows that are still true, fix the rest, keep a `Notes` section as it is. If `MAP.md` exists without that line, stop and ask: it is not ours.
4. Run `bash ${CLAUDE_SKILL_DIR}/scripts/map-tree.sh check <repo>`. Fix every `MISSING` line and run it again.
5. Add one line to the root `CLAUDE.md` (or `AGENTS.md` when there is no `CLAUDE.md`; create a `CLAUDE.md` with only this line when there is neither): `Read MAP.md first to find where things live.` Put it near the top, after the intro. Skip it when a line naming `MAP.md` is already there. When `AGENTS.md` is a link to `CLAUDE.md`, edit the one file. Change nothing else in it.
6. Do not commit or push. Report the shape below.

## MAP.md shape

```markdown
<!-- ps:map -->
# Map

<One sentence: what this repo is.>

## Folders
| Path | Holds | Read it when |
|---|---|---|
| `src/lib/` | pure logic, one file per feature | changing behavior |

## Find it fast
- <the thing people ask for, in plain words>: `path/to/file.ts`
(6 to 10 lines: entry points, the config file, the test setup, the build or deploy script, the place a new feature goes.)

## Commands
<One line: where the real list is, for example "see `package.json` scripts" or the check command.>

## Notes
(leave empty on the first write)
```

- A row per top-level folder (hidden ones too, when they hold something an agent edits), and a row for each subfolder the tree lists under a folder of 30 or more files.
- In backticks write only real repo paths. URLs, globs and bare extensions go in plain words: the check reads every backticked path.
- No file counts and no lists of every file: they go stale. Name a file only in "Find it fast".
- Long docs stay where they are: link them (`docs/architecture.md`), do not repeat them.

## Report

```
Map: MAP.md (<n> lines), check: ok
CLAUDE.md: line added | already there
Not committed.
```
