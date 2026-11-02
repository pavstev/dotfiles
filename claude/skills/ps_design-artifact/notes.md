# ps:design-artifact — reference

Contract for the single-file design-knobs artifact in [template.html](template.html). Smart defaults: shadcn/ui-style semantic OKLCH tokens, live WCAG contrast, multi-format export (CSS variables / Tailwind config / component props).

## DESIGN block

Edit only the block marked `// ==== EDIT THIS BLOCK ====`.

| Field | What it is |
|---|---|
| `title` | Artifact header + `<title>`. |
| `subtitle` | One line under the title. |
| `tokens` | Knob definitions. Each drives one CSS custom property on `:root`. |
| `variants` | Saved coherent starting points (`id`, `label`, `tokens`). First variant auto-loads. |
| `renderPreview(tokens)` | Returns HTML for the live preview (card + button + badge + input by default). Escape with `esc()`. |
| `pickPayload(tokens)` | Plain object copied into the `pick:` line. |

### Token types

- `range`: continuous — `min`, `max`, `step`, `value`, `unit` ("px","%",""), `css` (`--ds-radius`).
- `color`: hex — `value` (`#b4552d`), `css`.
- `select`: choice between kinds — `options: [{label,value}]`, `value`, `css`.
- `toggle`: on/off — `value: true|false`, `css`. Renders as a switch.

Every token writes to `document.documentElement.style.setProperty(css, value)`, so preview, chips and exports share one source of truth.

## Built-in behaviour

The features are listed in `SKILL.md`. Details: contrast is computed from sRGB luminance (`>= 4.5` passes); a variant loads its whole token set into every knob; the export panel appears after a pick (the Tailwind block is an `extend` block); dark mode uses `.dark` / `prefers-color-scheme: dark`.

## The pick line

```
pick: <id> { "radius": "12px", "primary": "#b4552d", ... }
```

"Copy my pick" writes it to the clipboard; a readonly fallback textarea appears if clipboard access is blocked.

## Apply it back in the project

The steps (echo, apply, delete on a yes) and "Rework" are in [ps:pick](../ps_pick/SKILL.md). Map each key to the project and give the exact snippet:

- CSS variables: `--ds-radius: 12px; --ds-primary: #b4552d;` in `:root` / `.dark`.
- Tailwind: add to `tailwind.config.js` `theme.extend.colors.primary` / `borderRadius.DEFAULT` (see the export panel).
- Component props: pass `radius="12px" primary="#b4552d"` etc.

## Sandbox constraints

The artifact works in Claude Code, Desktop, Web and iOS/Android. Phone width: single column, sticky footer, `env(safe-area-inset-bottom)`.

## References

- shadcn/ui theming — CSS variables → Tailwind utilities → components, semantic `name`/`name-foreground` pairs, OKLCH tokens, `--radius` derived scale — https://ui.shadcn.com/docs/theming
- shadcn/ui customization — presets, custom colors, cva variants, wrapper components — https://github.com/shadcn-ui/ui/blob/main/skills/shadcn/customization.md
- WCAG 2 contrast — minimum 4.5:1 for normal text, 3:1 for large text — https://www.w3.org/WAI/WCAG21/quickref/#contrast-minimum
- OKLCH color space — perceptually uniform, used by shadcn/ui and modern design systems — https://developer.mozilla.org/en-US/docs/Web/CSS/color_value/oklch
- Claude Artifacts — sandboxed iframe, CDN-restricted (cdnjs only for React artifacts; this file uses none), no localStorage — https://support.claude.com/en/articles/14604397-set-up-your-design-system-in-claude-design
