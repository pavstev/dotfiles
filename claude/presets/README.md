# Presets

Opt-in working habits. Each folder is one preset:

- `rules.md`: the rules. Front matter holds `name` (the folder name), `description` (one line) and
  `guard:` (the guard check ids the preset owns; the guard runs a check only while its preset is on).
- `settings.json` (optional): a fragment merged into `~/.claude/settings.json` while the preset is on.

`dots` links each enabled preset to `~/.claude/rules/preset-<name>.md` and merges its fragment
between the repo base and the private layer (`~/.config/dotfiles/claude-settings.json`). A preset
that is off costs no tokens. The core rules stay in `claude/CLAUDE.md`.

| Preset | What it does | Guard | Settings |
|--------|--------------|-------|----------|
| `workflow` | Speed over ceremony, hand-offs in files, helper briefs, question style and a fixed set of agents | G1-G7, G12, G13, G18, G19, G20, H1, W5 | none |
| `pace` | Make the weekly usage budget last the work days, with a cap on helpers at once | G8, PACE | none |
| `models` | Effort and model picks for the main thread, helpers and hand-offs | G9, ROUTER | `model`, `env.CLAUDE_CODE_SUBAGENT_MODEL` |
| `clock` | Say the local time and an estimate with a range before and after each phase | G11 | none |
| `plain-writing` | Short plain sentences, no em dashes, no taglines | none | `outputStyle: ELI5` |
| `no-ai-attribution` | Never add the assistant as co-author or add a "generated with" line | G17 | empty `attribution` |
| `ship-to-main` | Work lands on main in one commit and gets pushed in the repos of `SHIP_OWNERS`; the "archive this" routine | G10 | none |

The "What it does" column quotes each `rules.md` `description:`, which is the source. A new
install gets every preset on except `ship-to-main`. Change the list with `dots settings`
(`PRESETS`).

## Add a preset

1. Make `claude/presets/<name>/rules.md`, at most 30 lines, with the three front matter keys.
2. Write it in neutral words: no city, plan name, weekday list or model version number; refer to
   other rules by name, never by number.
3. Add `settings.json` only when the preset needs a setting.
4. Run `cargo test --test presets` in `rust/agent-config`.
