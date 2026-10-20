---
name: pace
description: Make the weekly usage budget last the work days, with a cap on helpers at once
guard: [G8, PACE]
---
# Pace

- **The weekly budget must last all the work days** (`WORK_DAYS` in `dots settings`, default mon-fri). Fewer, smaller helpers; run helpers in parallel only for independent pieces.
- **Helper cap:** at most 3 helpers at once, 1 when the week runs ahead of pace, none when it is critical. The guard enforces it.
- When the status line says the week runs ahead, drop optional work: no extra review rounds, no sweeps, a lighter plan.
