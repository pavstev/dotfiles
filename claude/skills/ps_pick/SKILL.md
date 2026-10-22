---
name: ps:pick
description: "Builds one lightweight artifact with numbered design or configuration variants, live knobs and a Copy-my-pick button, then applies the pick the user pastes back. Use when the user wants to see options to choose from, asks for mockups, examples, N variants, knobs or sliders to compare, or says they cannot judge from descriptions."
argument-hint: "[what to choose between, and how many options]"
---

# Pick

Input: $ARGUMENTS

The user cannot judge from words, so show the options. One artifact, one click to pick, one paste back.

## Build

If you must ask first (what to choose, how many options, which folder), put up to 4 independent questions in one question-tool card, recommended option first. Default the rest and say so in one line.

1. Load the `artifact-design` skill first. It holds the page contract.
2. Copy [template.html](template.html) to the scratchpad and fill its `VARIANTS` array. Keep the rest of the file as is.
3. Publish it with the Artifact tool (`icon: "palette"` or similar). One artifact, one file. Never several files, never prose-only options.

## What the artifact is

In this order:

1. A one-line title and one line saying what is being chosen.
2. Variant cards, grouped. Each card has its id, a short label, a live preview, and a "Pick this" button.
3. A sticky footer with the current pick and a "Copy my pick" button.

Rules for the cards:

- At most 20 variants. Fewer is better.
- Ids are a letter for the group plus a number: A1, A2, B3. Ids never change after publish.
- Show the real thing. Fonts render in that font, colors are swatches, layouts are small live layouts.
- Add knobs (slider, color input) only for continuous values such as size, radius, hue. Never for choices between kinds.
- Light and dark both work. Phone width works.

## What the reply is

The link, then a legend of three lines. Nothing else.

```
A1-A3: <what group A varies>. B1-B3: <what group B varies>.
Click "Pick this" on a card, adjust the knobs, then "Copy my pick".
Paste the line back here. It looks like: pick: A2 {"size":18}
```

## Apply the pick

When the user pastes `pick: <id> {json}`:

1. Echo one line: the id, its label, and the knob values.
2. Apply it to the project. The json is the full spec: label, variant data and knob values. If the project is not this session's folder, give the exact snippet and ask for its path.
3. Offer to delete the artifact. Delete only on a yes.

If the user gives only a number or a name, map it to the id, echo it, and go on.

## Rework

When the user wants a change after seeing the options, change only the one thing they named. Keep every other id and card the same. Republish to the same URL. Add new variants with new ids. Never renumber.
