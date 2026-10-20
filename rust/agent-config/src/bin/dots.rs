//! The `dots` command: pulls the repo, links files and runs the provisioning steps.
//! The `dotfiles` shim builds and runs it; the work is in `agent_config::dots`.
fn main() { std::process::exit(agent_config::dots::main()); }
