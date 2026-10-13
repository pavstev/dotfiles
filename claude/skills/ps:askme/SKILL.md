---
name: ps:askme
description: Use when the user runs /ps:askme, appends /ps:askme to a request, or explicitly asks to be grilled, interviewed, or questioned about a request, plan, or design before any work starts.
argument-hint: "[request to stress-test — empty = your last message]"
---

# ps:askme — adaptive pre-build interview

<purpose>
Interview me before any work starts, until we explicitly agree on requirements, edge cases, and architecture.
A wrong assumption caught now costs one question; caught after implementation, it costs a rewrite.
This is a conversation that adapts, not a fixed questionnaire: every question should be the single most useful one given everything I have already told you.
</purpose>

<request>
The request is the text passed with this command (shown as ARGUMENTS, or after the command name).
If nothing was passed, it is my most recent message before the command.
If neither gives you a request, ask me what to stress-test.
</request>

<light_mode>
When /ps:askme is appended to a request or used mid-task: recon fast, no decision map, no status line.
Ask only blockers (data model, public interface, costly rework), all in one `AskUserQuestion` call, 2-4 options each, recommendation first.
Take defaults for the rest and name them in one line. After the answers, start work.
</light_mode>

<asking>
Ask with your structured question tool: `AskUserQuestion` in Claude Code, `question` in OpenCode.
If neither exists, ask in plain text with lettered options and wait for my reply.

- Give 2–4 technically distinct options per question. Build them from this project's real context (its stack, existing patterns, the files you read), not textbook defaults.
- Keep labels short. In each description, state what the option means and its concrete trade-off, e.g. "ships today, but every request scans the full table".
- Put your recommendation first, with "(Recommended)" at the end of its label.
- Use multi-select only when options can combine. In Claude Code, use `preview` to compare code, schemas, or layouts side by side.
- The tool adds a free-text answer automatically, so every option you write is a real choice.
- One topic per call. Questions in the same call must be independent: when question B depends on how I answer A, ask A alone first.
</asking>

<phases>

## 1. Recon
Start with infra facts (plan tier, domain, project, branch). Then read what you need: the relevant code, config, and docs, plus the web when the request touches external APIs or libraries.
Look facts up instead of asking me. Spend my attention only on decisions that are mine: preferences, priorities, trade-offs, business rules, and constraints you cannot see.
Stay read-only for the whole interview. Leave files and state untouched until I approve the plan, because changes made before the key decisions are usually wrong.

## 2. Map
Build a decision map: every open decision, tagged **high / medium / low** by cost of being wrong, with dependencies between them.
High means data model, public interfaces, security boundaries, external contracts, or irreversible operations.
Show me the map once, briefly (one line per item, about 8 lines at most).
Start with the highest-impact decision whose prerequisites are settled.

## 3. Interview loop
Each round:
1. One status line, e.g. `Locked 4 · Open: 2 high, 3 low`.
2. In 1–2 sentences, name the risk behind the next question and why it matters. Act as a skeptical senior reviewer: look for unstated assumptions, edge cases (empty, huge, concurrent, failed, partially failed, retried), security holes, data-loss risk, scaling limits, and conflicts with existing code. When my idea has a flaw, say so plainly.
3. Ask.
4. Process my answer with `<after_each_answer>` before choosing the next question.

</phases>

<after_each_answer>
Every answer changes the map. Work through these in order:

1. **Unclear?** The answer is vague or hedged ("probably", "it depends", "whatever's standard"), answers only part of the question, or is free text with more than one reading. Then the next question drills into that same answer: quote my words, state the readings you see, and offer them as options. Move on only once the meaning is unambiguous.
2. **Contradiction?** It conflicts with an earlier answer or with what the code does. Show both side by side and ask which one wins.
3. **Against your recommendation?** I chose an option you advised against and it carries a real risk. Name that specific risk once and ask me to confirm or switch. If I confirm, accept it, record it as an accepted risk, and move on. Challenge once; never nag.
4. **Ripple.** Work out what the answer implies: decisions it opens (e.g. "offline support" opens sync conflicts), decisions it closes or makes moot, and new risks it creates. Update the map and re-rank.
5. **Next.** Pick the highest-impact open decision. When my answer just opened something high-impact, follow up on it now, while the context is fresh. Skip anything my answers already settled.
</after_each_answer>

<stop_condition>
Stop when either:
- every open item is low-impact, meaning it would not change the data model, interfaces, security, or overall approach. Decide those yourself and list them as defaults; or
- I say "go", "enough", "ship it", or similar. Then list the unresolved items as explicit assumptions.
</stop_condition>

<exit>
1. Write the decision record:
   - **Goal** — one sentence
   - **Decisions** — each with its reason and the alternative rejected
   - **Defaults & assumptions** — what you decided without me
   - **Accepted risks** — where I overrode your recommendation
   - **Out of scope**
2. Ask one final question: implement now, revise a decision, or save the record as a spec file and stop. Implement only after I choose.
</exit>

Light mode: ask now. Otherwise begin with recon, then show the map and ask the first question.
