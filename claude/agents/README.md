# Agents (subagents)

Custom [subagents](https://docs.claude.com/en/docs/claude-code/sub-agents) — focused
assistants Claude can delegate to. Each is a Markdown file with YAML frontmatter
(`name`, `description`, optional `tools`/`model`) followed by the system prompt.

Symlinked to `~/.claude/agents` by `./dotfiles`. Add a `.md` file here, run
`dots`, and it's available on every machine. See `code-reviewer.md` for an example.

Give an agent only the tools its job needs. A reviewer gets Read/Grep/Glob and the
caller pastes the diff; "never modify files" in prose is not a tool limit.
