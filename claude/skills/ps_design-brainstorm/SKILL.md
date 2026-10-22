---
name: ps:design-brainstorm
description: "Interviews the user about a design goal, does online research on the domain, asks clarifying questions in a loop (ps:askme conventions), builds a live-knob artifact via ps:design-artifact, and repeats until the user is satisfied. Use when the user wants to explore a UI design together before anything is built."
argument-hint: "[what to design — e.g. a dashboard card, a checkout form, a settings modal]"
---

# Design brainstorm

Input: $ARGUMENTS

Explore a UI design together: research the domain, interview in a loop, build a live-knob artifact, revise, repeat until you say ship it.

## Steps

### 1. Recon
Restate the goal in one sentence. Name the domain and the part of the UI it touches. Read any existing design-system files in this session's folder (`globals.css`, `tailwind.config.js`, component files) so tokens and variants match the real stack — not textbook defaults. Stay read-only until you approve the plan.

### 2. Research
Do online research on the domain (the component kind, accessibility norms, common token choices). Ground your suggestions in what you find; cite the source briefly. Do not ask me things you can look up.

### 3. Decision map
List the open decisions, each tagged high / medium / low by cost of being wrong:
- what component to build (high)
- which tokens vary it (radius, primary, shadow, text size, border, surface) (medium)
- who the audience is and the constraints (mobile, dark mode, WCAG) (high)
- saved variants to offer (low)

Show the map once, briefly (one line per item, ~8 lines max). Start with the highest-impact decision whose prerequisites are settled.

### 4. Interview loop (ps:askme conventions)
Each round:
1. One status line, e.g. `Locked 3 · Open: 1 high, 2 medium`.
2. In 1–2 sentences, name the risk behind the next question and why it matters. Act as a skeptical senior reviewer: flag unstated assumptions, edge cases (empty, huge, concurrent, failed), security holes, data-loss risk, scaling limits, and conflicts with existing code. When my idea has a flaw, say so plainly.
3. Ask one question with the structured question tool (AskUserQuestion in Claude Code, AskQuestion in Cursor, `question` in OpenCode): 2–4 technically distinct options, labels short, recommendation first with "(Recommended)", meaning + trade-off in each description. One topic per call; dependent questions go one at a time.
4. Process the answer (unclear → drill in; contradiction → show both and ask which wins; against recommendation → name the risk once, challenge once, never nag; ripple → update the map and re-rank). Then pick the next highest-impact open decision.

Stop when every open item is low-impact, or you say "go", "enough", "ship it", or similar. List unresolved items as explicit assumptions.

### 5. Build the artifact (ps:design-artifact)
Invoke `ps:design-artifact` with a filled `DESIGN` block matching our decisions:
- `title` / `subtitle` from the goal
- `tokens`: the locked range/color/select/toggle tokens, each writing a `--ds-*` CSS custom property
- `variants`: 2–4 coherent saved starting points (A1, A2, …)
- `renderPreview(tokens)`: the live preview (card + button + badge + input by default)
- `pickPayload(tokens)`: the object copied into the `pick:` line

Publish it as one artifact. Zero external dependencies. Light + dark, phone width, WCAG contrast badges live.

### 6. Review & loop
Show the artifact link, then ask: satisfied / revise a decision / change the build.
- Revise a decision → return to step 4, change only that decision, keep every id and variant the same, republish to the same URL.
- Change the build → adjust `renderPreview` / tokens and republish.
- Satisfied → done.

Never renumber ids or variants between rounds.

### 7. Exit
Write the decision record:
- **Goal** — one sentence
- **Decisions** — each with its reason and the alternative rejected
- **Defaults & assumptions** — what I decided without you
- **Accepted risks** — where you overrode my recommendation
- **Out of scope**

Then one final question: implement now (apply the `pick:` values to the project), revise a decision, or save the record as a spec file and stop. Implement only after you choose.

## Rules

- Research before asking: spend my attention only on decisions that are mine.
- One question per call; options technically distinct; recommendation first.
- Locked decisions stay locked unless you ask to reopen them.
- One artifact, one file, republished to the same URL across rounds. Never renumber.
- Long reference material lives in [notes.md](notes.md).
