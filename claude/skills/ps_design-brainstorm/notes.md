# ps:design-brainstorm — reference

Research guidance and the pattern the agent uses to build the `DESIGN` block for `ps:design-artifact` after each interview round.

## Research to do before asking

Look up, don't ask:
- The component kind's standard anatomy and states (default, hover, focus, disabled, empty).
- Accessibility norms for it (WCAG contrast 4.5:1 normal / 3:1 large; focus-visible outline; color not the only carrier of meaning).
- Common token choices for the domain (radius, primary hue, shadow, text size, border/surface).
- Mobile constraints (min touch target 44 px, readable min font 14–16 px).

Cite the source briefly in your answer (one line, e.g. "WCAG AA requires 4.5:1 for normal text — w3.org/WAI/WCAG21/quickref/#contrast-minimum").

## Building the DESIGN block

After the interview locks decisions, generate this exact shape inside `template.html`'s `DESIGN` block:

```js
const DESIGN = {
  title: "<one sentence goal>",
  subtitle: "<domain + constraints>",
  tokens: [
    { key: "radius", label: "Corner radius", type: "range", min: 0, max: 40, step: 1, value: 12, unit: "px", css: "--ds-radius" },
    { key: "primary", label: "Primary color", type: "color", value: "#b4552d", css: "--ds-primary" },
    { key: "shadow", label: "Shadow", type: "toggle", value: true, css: "--ds-shadow" },
    { key: "textSize", label: "Text size", type: "range", min: 12, max: 24, step: 1, value: 16, unit: "px", css: "--ds-text-base" },
    { key: "lineColor", label: "Border color", type: "color", value: "#ddd5cb", css: "--ds-line" },
    { key: "surface", label: "Surface color", type: "color", value: "#ffffff", css: "--ds-card" }
  ],
  variants: [
    { id: "A1", label: "<name>", tokens: { radius: 12, primary: "#b4552d", shadow: true, textSize: 16, lineColor: "#ddd5cb", surface: "#ffffff" } },
    { id: "A2", label: "<name>", tokens: { radius: 8, primary: "#2563eb", shadow: false, textSize: 14, lineColor: "#cbd5e1", surface: "#f8fafc" } }
  ],
  renderPreview: (t) => { /* HTML string using var(--ds-*) tokens */ },
  pickPayload: (t) => ({ radius: t.radius + "px", primary: t.primary, ... })
};
```

Token type reference:
- `range`: continuous — min, max, step, value, unit, css.
- `color`: hex — value, css.
- `select`: choice between kinds — options [{label,value}], value, css.
- `toggle`: on/off — value: true|false, css.

`renderPreview` returns an HTML string; escape user text with `esc()`. Use `var(--ds-*)` throughout so preview, chips, exports and contrast badges share one source of truth.

`pickPayload` returns the plain object copied into the `pick:` line (e.g. `pick: A1 { "radius": "12px", ... }`).

## Applying the pick back to the project

When you paste `pick: <id> {json}`:
1. Echo: `<id> <label> → <json values>`.
2. Map each key to the project — give the exact snippet:
   - CSS variables: `--ds-radius: 12px; --ds-primary: #b4552d;` in `:root` / `.dark`.
   - Tailwind: `theme.extend.colors.primary` / `borderRadius.DEFAULT` (use the artifact's export panel).
   - Component props: `radius="12px" primary="#b4552d"` etc.
3. Ask for the file path if it is not this session's folder.
4. Offer to delete the artifact. Delete only on a yes.

## References

- ps:askme interview conventions — https://code.claude.com/docs/en/skills (asking phase, stop condition, exit)
- ps:design-artifact skill — sibling skill in this repo (template.html + notes.md)
- shadcn/ui theming — CSS variables → Tailwind utilities → components, OKLCH tokens — https://ui.shadcn.com/docs/theming
- WCAG 2 contrast — 4.5:1 normal / 3:1 large — https://www.w3.org/WAI/WCAG21/quickref/#contrast-minimum
- OKLCH color space — https://developer.mozilla.org/en-US/docs/Web/CSS/color_value/oklch
