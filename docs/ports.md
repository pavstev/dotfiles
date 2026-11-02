# Dev server ports in parallel worktrees

Many sessions of one repo each run in their own git worktree. A checked-in `.claude/launch.json`
gives every worktree the same port, so the dev servers clash. This page says how to avoid that.

## Findings (checked 2026-11-02)

Sources: [desktop](https://code.claude.com/docs/en/desktop.md) (Preview, "Configure preview servers"),
[worktrees](https://code.claude.com/docs/en/worktrees.md), [hooks](https://code.claude.com/docs/en/hooks.md).
No docs page names the `preview_start` tool. These notes describe the `launch.json` it reads.

- **Entry fields:** `name`, `runtimeExecutable`, `runtimeArgs`, `port`, `cwd`, `env`, `autoPort`,
  `program`, `args`, `url`. `port` defaults to 3000.
- **`autoPort` is the built-in answer.** `true`: the app finds a free port when the named one is
  busy. `false`: it fails instead. Unset: it takes the next free port, except for a port hard-coded
  inside an npm, yarn, pnpm or bun script. The server gets the port in the `PORT` variable, and the
  app rewrites `--port` or `-p` in `runtimeArgs` when it moves the server.
- **No `$PORT` or env substitution** in `launch.json` itself. A server must read `PORT` from its
  environment, or take the port from a `--port` flag in `runtimeArgs`. A shell string such as
  `bash -c "... ${PORT:-8787} ..."` expands at run time, so it works.
- **`preview_start` learns the port only** from the `port` field and these rules. The docs do not
  describe detecting the port the server really opened. A `url` must match the entry's `port`.
- **Hooks:** `WorktreeCreate` and `WorktreeRemove` exist. `WorktreeCreate` replaces the default git
  worktree step (the hook must make the worktree), and the docs do not say it fires for worktrees
  that the desktop Code tab makes. It is not a safe place for a port file.
- **`.worktreeinclude`** copies gitignored files into a new worktree. It cannot compute a port.
- **Not in the docs:** which `launch.json` a worktree session reads (its own copy or the main
  checkout's), whether `PORT` is set when `autoPort` is `false`, and whether `env.PORT` wins over the
  assigned port. This session did not test them live.
- **Live test (2026-11-02, website):** with 4321 held, a worktree session's `preview_start` got
  4322, and the app rewrote `--port 4321` in `runtimeArgs` to `--port 4322`. With the flag inside
  the `pnpm dev` script instead, the app could not rewrite it. Vite moved to 4322 by its own
  fallback, which can drift from the port the preview opens. So rule 4 below holds: move the flag
  into `runtimeArgs` (`pnpm exec astro dev --port 4321`).

## Decision

Use `autoPort: true` on each entry, and make the server follow the port:

1. The entry keeps its base port in `port`. A main checkout normally gets that port.
2. A server that takes a flag: put the flag in `runtimeArgs` (`--port 5175`). The app rewrites it.
3. A server that reads env: read `PORT`, or use `${PORT:-<base>}` inside a `bash -c` string.
4. A server that hard-codes its port in a script or config: change the script, or the entry cannot
   move. Such an entry stays "main checkout only".

Base ports are unique across the owner's repos (`~/work/*/.claude/launch.json`, and
`~/work/.claude/launch.json`). Check with:

```bash
jq -r '.configurations[] | "\(.port) \(input_filename) \(.name)"' ~/work/*/.claude/launch.json ~/work/.claude/launch.json | sort -n
```

Three cross-repo clashes were fixed on 2026-11-02 (ports 5173, 8787 and 4337): one side of each
moved to a free base port (5175, 8788 and 4338). The repo whose scripts and process manager name
its port kept it, so those scripts needed no change.

## The `port` helper (fallback)

`rust-bin port <name> <base> [--no-probe]` prints the port for a checkout:

- Main checkout, or outside a repo: the base port.
- Worktree (a `.git` file, not a folder): `base + 1 ..= base + 99`, picked from a hash of the server
  name and the worktree path. The same worktree and name give the same port. The pick skips ports
  that any `launch.json` in the folder above the repos claims, and ports that something listens on
  now (`--no-probe` skips that check, so the answer does not change while the server runs).
- Exit 0 with the port, 1 when no port is free, 2 on a usage error.

Use it only for a server that ignores `PORT`, takes no `--port` flag that `autoPort` can rewrite, and
runs without `preview_start` (a script, a test run, a terminal). `preview_start` knows only the entry's
`port` field, so a server that moved to the helper's port is invisible to the preview. Everywhere
else prefer `autoPort`: it needs no helper in the project.

## Rule when none of this fits

Run the dev server in the main checkout only. In a worktree use tests and headless checks.
