---
name: models
description: Effort and model picks for the main thread, helpers and hand-offs
guard: [G9, ROUTER]
---
# Models

- **Main thread:** in the desktop app, pick Opus at `high` (the picker has no `opusplan`); plans run on it. In the CLI use `opusplan`, which plans on Opus and builds on Sonnet.
- **Effort:** `high`. Never `xhigh` or `max` unless the user names it.
- **Hand-offs:** hand-off blocks and `start_session` use `high`: Opus for plans, Sonnet for builds.
- **Helpers:** the guard router sets model and effort. Plan and the final review run on Opus, lookups on Haiku, the rest on Sonnet at `high`. Never ask a helper for a higher tier than its job needs.
