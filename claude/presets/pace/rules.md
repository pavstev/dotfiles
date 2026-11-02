---
name: pace
description: Make the weekly usage budget last the work days, with a cap on helpers at once
guard: [G8, PACE]
---
# Pace

- **The weekly budget must last all the work days** (`WORK_DAYS` in `dots settings`, default mon-fri). Speed still comes first: run independent pieces in parallel.
- **Helper cap:** at most 3 helpers at once (superlite: 10), none when the week is critical. The guard enforces it.
- An ahead week is a note only: say it once and keep going. A critical week: finish the current step, commit it and spawn nothing new.

**Remember:** at most 3 helpers at once; an ahead week changes nothing; a critical week spawns nothing.
