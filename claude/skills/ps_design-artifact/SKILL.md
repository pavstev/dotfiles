---
name: ps:design-artifact
description: "Builds one self-contained HTML artifact with configurable knobs (sliders, color pickers, selects, toggles) for exploring and comparing design variations, with live WCAG contrast and CSS/Tailwind/props export. Use when the user wants to design a UI interactively, turn knobs to see the effect live, and copy the final values back into the project."
argument-hint: "[what to design, e.g. a card, a button, a dashboard widget]"
---

# Design artifact

One HTML artifact, live knobs, WCAG contrast, one `pick:` line to copy back into the project.

## Steps

1. Decide what to design and which design tokens vary it (corner radius, primary color, shadow, text size, border/surface color).
2. Copy [template.html](template.html) to the scratchpad and fill the `DESIGN` block: `title`, `tokens` (range/color/select/toggle), `renderPreview(tokens)` and `pickPayload(tokens)`. Add coherent `variants`. Keep everything else as is.
3. Publish it as one artifact (`icon: "palette"`). One artifact, one file. Zero external dependencies.
4. The user turns the knobs; the preview repaints live with WCAG AA contrast badges. When they like it, they click "Pick this" and copy `pick: <id> {json}`.
5. Apply the pick: echo the id and the JSON values, then write them into the project (CSS variables, Tailwind tokens, or component props). Offer to delete the artifact; delete only on a yes.

## What the artifact is

- Header: title + one line saying what is being designed.
- Live preview (card, button, badge, input) that repaints on every knob move.
- Knob panel: range, color, select and toggle knobs, each driving a CSS custom property on `:root` (light + dark).
- Saved variants (A1, A2, …) that load coherent token sets; chips show each token's value.
- Live WCAG contrast badges for "Primary on surface" and "Text on primary".
- Export panel: CSS variables, Tailwind config, component props JSON — each copyable.
- Sticky footer: current pick + "Copy my pick" button that writes `pick: <id> {json}` to the clipboard.

## What the reply is

The link, then a three-line legend. Nothing else.

```
<id>-<n>: <what this group varies>. Pick a card, turn the knobs, copy the line.
Paste it back: pick: <id> { "radius": "12px", "primary": "#b4552d" }
```

## Apply the pick

When the user pastes `pick: <id> {json}`:

1. Echo one line: the id, its label, and the knob values.
2. Apply the JSON to the project: set the matching CSS variables, Tailwind tokens, or component props. Give the exact snippet; ask for the file path if it is not this session's folder.
3. Offer to delete the artifact. Delete only on a yes.

## Rework

Change only the one thing the user named. Keep every id and card the same. Republish to the same URL. Add new variants with new ids. Never renumber.

## Rules

- Zero external dependencies: inline CSS + vanilla JS only. Artifact sandboxes restrict which CDN hosts load.
- Tokens are CSS custom properties (`--ds-*`) so light/dark and the whole page track them.
- Knobs are for continuous values (size, radius, hue). Use select/toggle for choices between kinds.
- At most 20 variants. Light and dark both work. Phone width works.
- Long reference material lives in [notes.md](notes.md).
