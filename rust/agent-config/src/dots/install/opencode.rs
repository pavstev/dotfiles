//! 58-opencode: Opencode (the `opencode` CLI), which step 12-tools installs from tools.list. Skips
//! when `opencode` is present; warns when that step could not install it. Its config comes from
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
        warn("Opencode is missing. The tools step installs it (tools.list); read its warning, then run dots again.");
    }
}
