//! The `agent-config` command: builds Cursor and Opencode config and the Claude settings file
//! from `claude/`. The `scripts/agent-config` shim and `dots` run it; the work is in the library.
fn main() {
    std::process::exit(agent_config::run());
}
