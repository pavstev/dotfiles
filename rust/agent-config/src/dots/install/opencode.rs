//! 58-opencode: Opencode (the `opencode` CLI) from Homebrew, already in the Brewfile. Skips
//! when `opencode` is present; warns when brew could not install it. Its config comes from
//! claude/ via `agent-config opencode` (wired into `dots`).

use super::super::os;
use crate::out::{self, skip, warn};
use std::path::Path;

pub fn step(_repo: &Path) {
    os::brew_env(); // on a fresh Linux the Homebrew from step 10 is not on PATH yet
    out::step("Opencode");
    if os::have("opencode") {
        skip("Opencode");
    } else {
        warn("Opencode is missing. The Brewfile installs it (brew \"opencode\"); run dots again.");
    }
}
