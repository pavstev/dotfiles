---
name: ps:bag-mode
description: Use when the user is about to close the laptop lid or leave with a Mac while Claude sessions, subagents, or background tasks keep running, or asks which tasks are heavy, how long they will stay heavy, how long the session still needs, or how to keep Claude alive in a bag with Amphetamine.
allowed-tools: "Bash(${CLAUDE_SKILL_DIR}/scripts/bag-check.sh *) Bash(python3 ${CLAUDE_SKILL_DIR}/../ps:whatsleft/scripts/whatsleft-scan.py *)"
---

# Bag Mode

Check if the Mac is safe to carry while Claude keeps working. Estimate how long the session still needs. Find the heavy tasks, cut the load.

## Steps

1. Get the ETA (see "Estimate the ETA" below).
2. Run `bash ${CLAUDE_SKILL_DIR}/scripts/bag-check.sh --eta <ETA>`. Example: `--eta 45m`.
3. List this session's own subagents and background tasks. Say which one is slowest.
4. Cut load. Do not start new subagents. Stop idle background tasks you started. Run the script with `--throttle` to move dev servers and Spotlight to background priority.
5. Run the check again. Show the report. Give the VERDICT.

## Estimate the ETA

ETA = time the session still needs. The shared scan from `ps:whatsleft` works it out from open rows and running agents:

`python3 ${CLAUDE_SKILL_DIR}/../ps:whatsleft/scripts/whatsleft-scan.py --eta`

It prints one line, like `ETA ~35 min (25-50)`. Pass the high end of the range to the check (`--eta 50m`); the script adds its own 25% buffer. If the user gave a time, use their time. If this session knows later phases or agent deadlines the scan cannot see, add them and say so. If the scan finds no session, ask the user for a time.

The check compares ETA to battery time. On battery with less time than ETA: verdict HIGH, plug in.

## Output shape

The script prints a markdown report: a title with a colour dot, a two-column Check table, a Heavy tasks table, and a last `VERDICT:` line. Paste it as is. Do not put it in a code block, so the tables render. Then add one line: what to do now. Add nothing else.

```
## 🎒 Bag Mode — 🟢 LOW

| Check | Status |
|---|---|
| 🔋 Power | 78% on Battery · 14h55m left |
| ☕ Amphetamine | ✅ ON |
| 🔥 Load | 2.42 / 10 cores · 73% memory free |
| ⏱ ETA | ~45m (+25% buffer = 56m) · ✅ battery ok |
| 🤖 Running | 4 agents · 2 dev servers · 16 node |

### 🏋 Heavy tasks (2)

| Task | CPU | Ends |
|---|---|---|
| 🖥  screen drawing | 36% | when the lid closes |
| ❔ Claude Helper | 16% | unknown |

VERDICT: 🟢 LOW - safe to close the lid.
```

Dots: 🟢 LOW, 🟡 MEDIUM, 🔴 HIGH.

## Read the kinds

| Kind | Why heavy | How long |
|---|---|---|
| dev server | Watches files, rebuilds | Never ends. Stop it or throttle it. |
| build/lint | Compiles code | Ends with the build. |
| Claude agent | Runs a task | Ends with its task. |
| screen drawing | Display is on | Stops when the lid closes. |
| Spotlight | Scans disk | Ends alone in minutes. |
| container VM | Runs Docker | Never ends. Stop if idle. |

## Before the lid closes

- Amphetamine must show ON in the card. If OFF, the verdict is HIGH: the Mac sleeps and Claude pauses.
- Amphetamine setting "Allow system to sleep when display is closed" must be OFF. The script cannot read it. Ask the user to check.
- Single-Use sessions end on a timer. Tell the user to set it longer than the ETA.
- Verdict HIGH from load: a hot Mac in a bag gets throttled by the heat. Wait, or throttle first.
- Keep the bag open at the top. Do not wrap the Mac in cloth.
- Wi-Fi must stay on. Claude needs the network. A phone hotspot works.

## Undo

`bash ${CLAUDE_SKILL_DIR}/scripts/bag-check.sh --restore` puts throttled processes back to normal.

## Other modes

`--watch 60` repeats the report each 60 seconds. The ETA counts down. Mix with ETA: `--eta 45m --watch 60`.
