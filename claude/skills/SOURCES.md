# Skill sources & provenance

The seeded skills were authored for this repo, informed by patterns from the public
Claude Code / agent-skills ecosystem. They are starting points — adapt freely.

References consulted (June 2026):

- Anthropic official plugins/skills — https://github.com/anthropics/claude-plugins-official
- awesome-claude-skills — https://github.com/travisvn/awesome-claude-skills
- agent-skills open standard — https://agentskills.io
- Claude Code Skills docs — https://docs.claude.com/en/docs/claude-code/skills

The dev-workflow seeds from that sweep (`ps:go-lint`, `ps:readme-writer`) were deleted on
2026-10-12 for lack of use. The rest were written for this setup. Each skill is checked by the `skill_check` bin against:

- Claude Code skills reference — https://code.claude.com/docs/en/skills
- Anthropic skill authoring best practices — https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices
- Agent Skills spec — https://agentskills.io/specification
- superpowers `writing-skills` — description = triggers only; recipe over prohibition; test with subagents

The test runs behind each skill change (baseline, edit, re-run) were logged in this file until
2026-10-21. They now stay out of git, in the ignored `.agents/` folder; git history has the old log.

## Ideas per skill

superpowers was retired on 2026-10-22 for [superlite](https://github.com/iha2/superlite). The superpowers
credits below stay as history: those ideas came from it.


- **ps:whatsleft** (it took in ps:hurry-up on 2026-10-12; the speed scan is the `hurry_scan` bin):
  superpowers `verification-before-completion` and `writing-skills`, and these public ideas,
  rewritten for this setup:
  - context-budget (ECC) — inventory and flag bloat in agents, skills, rules, MCP — https://claudeskills.info/skills/affaan-m/ecc/context-budget/
  - Cozempic diagnose — read the live transcript, show context %, biggest messages — https://openskillindex.com/skills/ruya-ai-cozempic-diagnose
  - Token Optimizer — audit setup, find context waste — https://vibeindex.ai/collection/alexgreensh/token-optimizer
  - "Making a bloated Claude Code fast again" — measure injected bytes, not file sizes — https://dev.to/bokuwalily/making-a-bloated-claude-code-fast-again-auditing-context-injection-down-from-228kb-to-48kb-1644
- **ps:mic-drop:** superpowers `verification-before-completion` (no "done" claim without fresh
  evidence) and `finishing-a-development-branch` (ask before merging to main).
- **ps:iamrich:** superpowers `writing-skills` (required fields over prose reminders) and
  `subagent-driven-development` (a fresh agent per task, a review between tasks), and
  `ps:captain-research` (fan out, check each load-bearing claim with a second agent).
- **ps:harness-optimize** (2026-10-14): Anthropic's Claude Code docs (memory, skills, hooks,
  prompt caching), the context-engineering and long-running-harness posts, skillscope (skill
  usage from Skill tool_use), and the AGENTS.md studies (arXiv 2602.11988, 2601.20404).
- **The fan-out gate, the hand-off State and ps:review-panel** (2026-10-15): four talks on
  "graph engineering": fan out only for independent parts (a multi-agent run costs about 15x a
  chat), a 4-question test before a graph, named state with per-node reads and writes, the judge
  node on the strong model, and fan-in at a barrier.
- **ps:standup, ps:draft-reply and the ps:review-panel risk lenses** (2026-10-20): three retired
  private repos of the repo owner (own work, no third-party text).

## Ideas from the engineering, design and desktop-commander plugins (2026-10-19)

Reviewed 23 skills from three synced plugins before turning them off (13 calls in 19 days, none for
desktop-commander). Ideas only, rewritten for this setup, no text copied. Full review in the ignored
`.agents/research/2026-10-19-learn/plugin-skills-review.md`.

- engineering `code-review` (Anthropic, Apache-2.0, https://github.com/anthropics/knowledge-work-plugins): the security, performance and correctness scan list, now one line in the `code-reviewer` agent.
- design `accessibility-review` (Anthropic, Apache-2.0, https://github.com/anthropics/knowledge-work-plugins): the label, keyboard, focus, target size, contrast and zoom checks in the `ui-verifier` agent.
- desktop-commander `terminal` and `desktop-commander-overview` (Desktop Commander, MIT, https://github.com/wonderwhy-er/DesktopCommanderMCP): the persistent interactive-session pattern, now driven through tmux in `ps:autopilot`. The desktop-commander MCP tools were removed from `ps:autopilot`, `ps:whatsleft` and `ps:mic-drop`.
- Not taken: the other 20 skills (generic templates, connector flows, Obsidian, Hermes and OpenClaw setup).

## Ideas from AI-coding videos and articles (2026-10-22)

Ideas only, rewritten for this setup, no text copied.

- **ps:harness-optimize prune test:** "could Claude find this in the repo?" (arXiv 2602.11988: context files that restate the repo raise cost by 20% or more). The A/B mode uses `claude --safe-mode` as the control arm.
- **ps:map:** a short folder map that the root `CLAUDE.md` points at (an article reported 79% faster work overall in its own test; our numbers are in the commit message).
- **ps:autopilot:** a done marker before each push or deploy (idempotent resume) and a `Budget:` line (stop on a limit), from graph-engineering videos.
- **ps:review-panel:** the reviewer budget (3 rounds, stop at nits, correctness and stated requirements only).
- **ps:design-ref:** Dembrandt (MIT, https://github.com/dembrandt/dembrandt) for DESIGN.md and the drift
  check, and the Awwwards MCP (MIT, https://github.com/insane0777/awwwards-mcp) for picking reference sites,
  from an AI LABS video on design repos. Both are installed once from `tools.list`; the MCP is added per
  project only, so it costs no tokens elsewhere.
