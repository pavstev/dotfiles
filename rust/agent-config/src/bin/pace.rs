//! `pace status | refresh | seed <percent>`: the CLI of claude/hooks/pace.py.
#[path = "../pace_core.rs"]
mod pace;

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect();
    let cmd = args.get(1).map_or("", String::as_str);
    match (cmd, args.len()) {
        ("status", 2) => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            if pace::needs_refresh(now) {
                pace::refresh(5.0, Some(now));
            }
            println!("{}", pace::status_json(&pace::status(Some(now))));
            ExitCode::SUCCESS
        }
        ("refresh", 2) => {
            println!("{}", pace::refresh_json(&pace::refresh(120.0, None)));
            ExitCode::SUCCESS
        }
        ("seed", 3) => {
            match pace::parse_percent(&args[2]) {
                Some(used) if pace::seed(used, None) => {
                    println!("{}", pace::seed_json(pace::week_weight().unwrap_or(0.0)));
                    ExitCode::SUCCESS
                }
                _ => {
                    eprintln!("pace: seed needs a percent above 0 up to 100, and some usage this week");
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!("usage: pace status | refresh | seed <percent>");
            eprintln!("the plan and the work days come from PLAN and WORK_DAYS in ~/.config/dotfiles/settings");
            ExitCode::from(2)
        }
    }
}
