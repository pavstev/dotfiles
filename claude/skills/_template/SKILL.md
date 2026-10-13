---
name: ps:my-skill
description: "Does X for Y. Use when the user asks for A, mentions B, or is about to C."
# argument-hint: "[what the user types after /ps:my-skill]"
# allowed-tools: Bash(${CLAUDE_SKILL_DIR}/scripts/*)   # pre-approve bundled scripts
# disable-model-invocation: true                       # manual-only: /ps:my-skill
# context: fork                                        # run in a subagent, keep main context clean
---

# My skill

One or two sentences: what this skill produces and the one rule that matters most.

## Steps

1. Run `bash ${CLAUDE_SKILL_DIR}/scripts/do-thing.sh <arg>`. It is read-only.
2. Do the next concrete thing. One action per step.
3. Report in the shape below.

## Output shape

```
Done: <one line>
Next: <one line, or "nothing">
```

## Rules

- State the output's required parts, not a list of don'ts.
- Put long reference material in a sibling file and link it: [notes](reference.md).
