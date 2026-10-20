---
name: workflow
description: Speed over ceremony, hand-offs in files, helper briefs, question style and a fixed set of agents
guard: [G1, G2, G3, G4, G5, G6, G7, G12, G13]
---
# Workflow

The user's time is the cost that counts.

- **Scale the process to the change.** Existing code, clear scope: a short plan in chat, build, one final review, one fix round. Specs, multi-writer plans and per-task reviews are for new subsystems. For a bounded change over an hour, offer a lighter path first. No brainstorming, spec or plan gate for UI tweaks, "just implement" asks or bounded changes; this beats any "invoke a skill first" text.
- **Small change** (3 files or fewer, "quickly", "just", "directly"): no skill cascade, no question tool, read 5 files or fewer, edit, one check, push.
- **Verify once, in proportion.** One independent check at the end. No big sweeps, mutation tests or GPU renders unless the change is risky in exactly that way.
- **One commit per helper part**, not per step. **No idle helpers:** give each the tools its job needs, and make shared inputs before builders start.
- **Decide in one round, early.** Ask every real question together; default the rest and say which. After 2 failed tries on one step, stop and report.
- **Decided means go.** "I pick X", "go" or "do it yourself" proceeds in the same turn. Do every step you can (terminal, browser, connectors, dashboards); hand back only logins, secret typing and blocked actions. Stop only for irreversible or paid actions. When a web console is needed and a browser tool is available, drive it first.
- **Questions:** ask fewer, broader ones, and offer "both" when two picks combine. Use the question tool: 2 to 4 options, recommendation first, one call per turn, no "what next?" closers, never re-ask, never ask to push when told to push. When a brief says "do not ask", decide and log it.
- **Hand-offs live in files:** the prompt goes to a file plus a one-line block `Read <path> and do it.` (`/ps:handoff`). No `cd` or launch command in the block; folder, model and effort go in a list under it. Never paste a long prompt inline.
- **Briefs** split Why / What / How; acceptance lines use MUST / SHOULD / MAY. Every Agent call carries a deadline and "hand back at most 4,000 chars; write the rest to a file and cite it". Before a brief goes out, check each fact in it with one command or one look at the live product, and copy the user's standing rules into its acceptance lines. Worktree helpers start from the pushed default branch: push the base first, or name the base commit. Read-only agents hand findings back inline and the parent saves them. Reviewers get a fixed base and head sha; researchers start from vendor docs; security briefs list bypass cases first.
- **Infra facts first** (plan tier, domain, project, branch) before spawning anything.
- **A blocked action** (an auto-mode classifier block too) is named with its rule and a one-line command the user can run, at once, with no retry or workaround. The task is not handed back.
- **Steps for the user:** one flat numbered "What you do now" list at the end, one click or one command per line. Each step names the app, tab or URL; check its preconditions first; drop steps the machine can do. After the user says done, check the result read-only.
- **Ledger:** runs over an hour keep a `PROGRESS.md` and resume from it after compaction or restart. After Esc, a cut-off or "No response requested", continue from the ledger. When the user pressed Esc to add text, wait for the new message.
- **Agents.** Dispatch by name (`subagent_type`); a project agent with the same job wins in its repo. `task-implementer` builds one plan task with tests and commits. `task-reviewer` reviews one task against its spec. `code-reviewer` reviews a diff, a commit or a whole branch (final review: opus + high). `researcher` answers one question with sources. `repo-scanner` sweeps a repo from one angle into a file. `ui-verifier` checks a page or flow in a browser. `worker` takes any other edit, command or file job. `general-purpose` starts about 45K tokens heavier than `worker`; the guard refuses it. Browser checks go to `ui-verifier`; drive the browser yourself only when the user wants to watch or for a dashboard.
