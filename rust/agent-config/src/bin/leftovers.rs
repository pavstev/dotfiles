//! leftovers: the optional work of one project, parked for later. The store and the format are in
//! `agent_config::leftovers` (`<main checkout>/.agents/leftovers.jsonl`, git-ignored).
//!
//! ```text
//! leftovers add TEXT [--repo PATH] [--tag T]...     park one
//! leftovers list [--repo PATH] [--all] [--json]     open items, oldest first
//! leftovers done ID [--note T] [--repo PATH]        close one as done
//! leftovers drop ID [--note T] [--repo PATH]        close one as not wanted
//! leftovers count [--repo PATH]                     the number of open items
//! leftovers path [--repo PATH]                      the store file
//! leftovers compact [--repo PATH]                   rewrite with open and recent items
//! leftovers import [FILE] [--map NAME=PATH]... [--dry-run]   split the old shared Markdown file
//! ```

use agent_config::leftovers::{self, Error, Event, OldItem, Status, Store};
use agent_config::paths;
use agent_config::quiet_println as println;
use clap::{Args, Parser, Subcommand};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "leftovers", about = "Optional work of one project, parked for later", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Which project: the folder to ask git about (default: the current folder).
#[derive(Args)]
struct Where {
    #[arg(long, value_name = "PATH")]
    repo: Option<PathBuf>,
}

impl Where {
    fn store(&self) -> Result<Store, Error> {
        let dir = self.repo.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        Store::at(&dir)
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Park one leftover.
    Add {
        /// What is left. Free text, so it may start with a dash.
        #[arg(allow_hyphen_values = true)]
        text: String,
        /// A tag (repeat for more).
        #[arg(long = "tag", value_name = "T")]
        tags: Vec<String>,
        #[command(flatten)]
        at: Where,
    },
    /// List the open leftovers, oldest first.
    List {
        /// Add the closed ones with their state.
        #[arg(long)]
        all: bool,
        /// Print a JSON array.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        at: Where,
    },
    /// Close one as done.
    Done {
        /// The id, or a unique prefix of 4 hex or more.
        id: String,
        #[arg(long, value_name = "T")]
        note: Option<String>,
        #[command(flatten)]
        at: Where,
    },
    /// Close one as not wanted.
    Drop {
        /// The id, or a unique prefix of 4 hex or more.
        id: String,
        #[arg(long, value_name = "T")]
        note: Option<String>,
        #[command(flatten)]
        at: Where,
    },
    /// Print the number of open leftovers.
    Count(Where),
    /// Print the store file.
    Path(Where),
    /// Rewrite the file with the open items and those closed in the last 30 days.
    Compact(Where),
    /// Split the old shared Markdown file into the stores of its projects.
    Import {
        /// The old file (default: ~/.claude/leftovers.md).
        file: Option<PathBuf>,
        /// Where a project lives: NAME=PATH (repeat for more).
        #[arg(long = "map", value_name = "NAME=PATH", value_parser = parse_map)]
        maps: Vec<(String, PathBuf)>,
        /// Show what would happen and change nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

fn parse_map(s: &str) -> Result<(String, PathBuf), String> {
    match s.split_once('=') {
        Some((name, path)) if !name.is_empty() && !path.is_empty() => Ok((name.to_string(), PathBuf::from(path))),
        _ => Err("expected NAME=PATH".to_string()),
    }
}

fn say_ignored(store: &Store) {
    println!("added .agents/ to {} (commit it)", store.root().join(".gitignore").display());
}

fn run(cmd: Cmd) -> Result<ExitCode, Error> {
    match cmd {
        Cmd::Add { text, tags, at } => {
            let store = at.store()?;
            let added = store.add(text.trim(), &tags)?;
            println!("added {} to {}", added.id, store.path().display());
            if added.ignore_added {
                say_ignored(&store);
            }
        }
        Cmd::List { all, json, at } => {
            let store = at.store()?;
            let items = store.items();
            if json {
                let shown: Vec<_> = items.iter().filter(|i| all || i.status == Status::Open).collect();
                println!("{}", serde_json::to_string(&shown).expect("items are plain data"));
            } else if !store.path().is_file() {
                println!("no leftovers here ({})", store.path().display());
            } else {
                for line in leftovers::list_lines(&items, all) {
                    println!("{line}");
                }
            }
        }
        Cmd::Done { id, note, at } => close(&at, &id, Status::Done, note.as_deref())?,
        Cmd::Drop { id, note, at } => close(&at, &id, Status::Dropped, note.as_deref())?,
        Cmd::Count(at) => println!("{}", at.store()?.open_items().len()),
        Cmd::Path(at) => println!("{}", at.store()?.path().display()),
        Cmd::Compact(at) => {
            let store = at.store()?;
            if store.path().is_file() {
                let (kept, removed) = store.compact()?;
                println!("compacted {}: kept {kept}, removed {removed}", store.path().display());
            } else {
                println!("no leftovers here ({})", store.path().display());
            }
        }
        Cmd::Import { file, maps, dry_run } => return import(file, &maps, dry_run),
    }
    Ok(ExitCode::SUCCESS)
}

fn close(at: &Where, prefix: &str, status: Status, note: Option<&str>) -> Result<(), Error> {
    let item = at.store()?.close(prefix, status, note)?;
    println!("{} {}", item.status, item.id);
    Ok(())
}

/// A project and the old items that belong to it.
struct Project<'a> {
    store: Store,
    items: Vec<&'a OldItem>,
}

/// The old items by project (keyed by main checkout), and those no project was found for.
#[derive(Default)]
struct Grouped<'a> {
    found: BTreeMap<PathBuf, Project<'a>>,
    lost: Vec<&'a OldItem>,
}

fn group<'a>(items: &'a [OldItem], maps: &[(String, PathBuf)], bases: &[PathBuf]) -> Grouped<'a> {
    let mut grouped = Grouped::default();
    for item in items {
        let store = leftovers::resolve_repo(&item.name, maps, bases).and_then(|dir| Store::at(&dir).ok());
        match store {
            Some(store) => {
                let key = store.root().to_path_buf();
                grouped.found.entry(key).or_insert_with(|| Project { store, items: Vec::new() }).items.push(item);
            }
            None => grouped.lost.push(item),
        }
    }
    grouped
}

/// Add the old items of one project, keeping their dates. An item that is already in (same date and
/// text) is skipped, so a second run adds nothing.
fn import_project(project: &Project, dry_run: bool) -> Result<(), Error> {
    let Project { store, items } = project;
    let have: HashSet<(String, String)> = store
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Add { at, text, .. } => Some((at, text)),
            _ => None,
        })
        .collect();
    let mut fresh = 0;
    for item in items {
        let at = format!("{}T00:00:00Z", item.date);
        if have.contains(&(at.clone(), item.text.clone())) {
            continue;
        }
        fresh += 1;
        if !dry_run {
            let id = leftovers::new_id(&at, &item.text);
            store.append(&Event::Add { id, at, text: item.text.clone(), tags: Vec::new(), from: item.name.clone() })?;
        }
    }
    let verb = if dry_run { "would import" } else { "imported" };
    println!("{}: {verb} {fresh} of {}", store.root().display(), items.len());
    if fresh > 0 && !dry_run && store.ensure_ignored()? {
        say_ignored(store);
    }
    Ok(())
}

fn import(file: Option<PathBuf>, maps: &[(String, PathBuf)], dry_run: bool) -> Result<ExitCode, Error> {
    let home = paths::home();
    let file = file.unwrap_or_else(|| home.join(".claude").join("leftovers.md"));
    let text = std::fs::read_to_string(&file).map_err(|e| Error::Io(file.clone(), e))?;
    let (items, odd) = leftovers::parse_old(&text);
    let bases = [home.clone(), home.join("work")];
    let grouped = group(&items, maps, &bases);
    for project in grouped.found.values() {
        import_project(project, dry_run)?;
    }
    let unresolved = grouped.lost.len() + odd.len();
    for item in &grouped.lost {
        println!("unresolved: {} · **{}** · {}", item.date, item.name, item.text);
    }
    for line in &odd {
        println!("unresolved: {line}");
    }
    if dry_run {
        println!("dry run: {} kept, {unresolved} unresolved", file.display());
    } else if unresolved == 0 {
        std::fs::remove_file(&file).map_err(|e| Error::Io(file.clone(), e))?;
        println!("removed {}", file.display());
    } else {
        println!("kept {}: {unresolved} unresolved (use --map NAME=PATH)", file.display());
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run(Cli::parse().cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("leftovers: {e}");
            ExitCode::from(e.code())
        }
    }
}
