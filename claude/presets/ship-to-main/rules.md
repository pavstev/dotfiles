---
name: ship-to-main
description: Work lands on main in one commit and gets pushed in your own repos; the "archive this" routine
guard: [G10]
---
# Ship to main

Off by default: a push to the main branch of a work repo can hurt. Turn it on in `dots settings`.

- **Push scope:** push to main only in a repo whose origin GitHub owner is in the `SHIP_OWNERS` setting (a space list in `dots settings`). In any other repo, or with an empty list, work on a branch and open a PR. A rule in the project's `CLAUDE.md` wins, for example "never push to main without asking".
- **Finishing:** work lands on main in one commit unless the user says PR. When asked, squash the session's commits (`/ps:mic-drop` has the steps).
- **Before a push:** `git fetch`, rebase on `origin/main`, run the fast check again, then push. A conflict you cannot solve: stop and report.
- **"Archive this"** means: commit, push (by the scope above), scan once, archive, in one go. No new verification and no "Archive now?". When the work is pushed and nothing waits on the user, archive the same way.
