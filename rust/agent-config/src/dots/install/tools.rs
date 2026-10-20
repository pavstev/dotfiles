//! 12-tools: the command-line tools of tools.list, on every OS. mise goes in first (no Homebrew,
//! no cargo), then each missing tool from the first source that works. See tools_cli.rs.
//! `dots` runs it once more at the end of a run, so a source that a later step installed (cargo
//! from step 40) is tried too.

use std::path::Path;

pub fn step(_repo: &Path) { crate::tools_cli::run(Vec::new()); }
