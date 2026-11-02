//! `port`: the dev server port of a checkout.
//!
//! ```text
//! port NAME BASE [--no-probe]   print the port for server NAME
//! ```
//!
//! In a main checkout (or outside a repo) it prints BASE. In a worktree it prints a stable port
//! above BASE (see `port.rs` for the pick). `--no-probe` skips the check for a port in use, so the
//! answer does not change while the worktree's own server runs.
//!
//! Exit 0 with the port on stdout, 1 when a worktree has no free port, 2 on a usage error.

use agent_config::port;
use clap::Parser;
use std::process::exit;

#[derive(Parser)]
#[command(name = "port", about = "Print the dev server port for this checkout")]
struct Cli {
    /// Name of the server (the `name` of its launch.json entry)
    name: String,
    /// Port the server uses in the main checkout
    #[arg(value_parser = clap::value_parser!(u16).range(1..))]
    base: u16,
    /// Do not skip ports that something listens on now
    #[arg(long = "no-probe")]
    no_probe: bool,
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            exit(if e.use_stderr() { 2 } else { 0 });
        }
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    if let Some(p) = port::resolve(&cwd, &cli.name, cli.base, !cli.no_probe) {
        println!("{p}");
    } else {
        eprintln!("port: no free port above {} for {}", cli.base, cli.name);
        exit(1);
    }
}
