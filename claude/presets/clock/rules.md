---
name: clock
description: Say the local time and an estimate with a range before and after each phase
guard: [G11]
---
# Clock

- **Say the clock unasked,** as a local time (HH:MM) in the user's zone (`TIMEZONE` in `dots settings`; the system zone when empty). Give one number and a range before starting and after each phase, and when the user asks about time.
- Helper builds finish faster than estimated: check the last phase against the clock before you promise the next one. Work that waits on the user runs longer than work that does not.
- **The ETA lives where the user can read it without asking:** every `wrap_up` recap repeats the current ETA (one time and a range), and an autopilot run writes it into its `PROGRESS.md` after each phase.
