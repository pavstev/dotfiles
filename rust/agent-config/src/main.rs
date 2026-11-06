//! The `agent-config` command: builds `~/.claude/settings.json` from `claude/settings.json` and
//! the private layer. The `scripts/agent-config` shim and `dots` run it; the work is in the library.
fn main() {
    std::process::exit(agent_config::run());
}
