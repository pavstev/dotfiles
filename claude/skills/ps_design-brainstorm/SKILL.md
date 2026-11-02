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
Run the loop of [ps:askme](../ps_askme/SKILL.md): its phase 3 (status line, risk sentence, batched questions with the recommendation first), `<after_each_answer>` and `<stop_condition>`. The map above is the decision map. Design adds one thing: question the tokens and the audience before the variants.

### 5. Build the artifact (ps:design-artifact)
Invoke `ps:design-artifact` with a filled `DESIGN` block matching our decisions:
- `title` / `subtitle` from the goal
- `tokens`: the locked range/color/select/toggle tokens, each writing a `--ds-*` CSS custom property
- `variants`: 2–4 coherent saved starting points (A1, A2, …)
- `renderPreview(tokens)`: the live preview (card + button + badge + input by default)
- `pickPayload(tokens)`: the object copied into the `pick:` line

Publish it as one artifact; the `ps:design-artifact` rules apply.

### 6. Review & loop
Show the artifact link, then ask: satisfied / revise a decision / change the build.
- Revise a decision → return to step 4, change only that decision, keep every id and variant the same, republish to the same URL.
- Change the build → adjust `renderPreview` / tokens and republish.
- Satisfied → done.

Never renumber ids or variants between rounds.

### 7. Exit
Exit as ps:askme's `<exit>` does: the decision record, then the final question. "Implement now" here means apply the `pick:` values to the project (ps:pick, "Apply the pick").

## Rules

- Research before asking: spend my attention only on decisions that are mine.
- Locked decisions stay locked unless you ask to reopen them.
- Long reference material lives in [notes.md](notes.md).
