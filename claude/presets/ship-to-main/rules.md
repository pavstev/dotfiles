---
name: ship-to-main
description: Work lands on main in one commit and gets pushed; the "archive this" routine
guard: [G10]
---
# Ship to main

Off by default: a push to the main branch of a work repo can hurt. Turn it on in `dots settings` for repos you own.

- **Finishing:** work lands on main in one commit unless the user says PR. When asked, squash the session's commits and `push --force-with-lease`.
- **"Archive this"** means: commit, push, scan once, archive, in one go. No new verification and no "Archive now?". When the work is pushed and nothing waits on the user, archive the same way.
