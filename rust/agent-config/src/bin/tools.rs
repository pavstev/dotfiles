//! `tools [--check] [--list] [--only a,b] [--host NAME] [--file tools.list] [-q] [-v]`: installs the
//! command-line tools of `tools.list` for this OS. `scripts/tools` runs it. See `tools_cli.rs`.
fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(agent_config::tools_cli::run(std::env::args().skip(1).collect()))
}
