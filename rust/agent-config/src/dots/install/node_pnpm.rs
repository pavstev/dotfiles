//! 50-node-pnpm: Node LTS as mise's global default. Installs only what is missing. Tolerant: a
//! failed network step warns and goes on. Alpine skips it: Node from mise needs glibc libraries
//! or a compiler there. pnpm is a tool of its own in `tools.list` (a standalone binary): Node 25
//! and up ship no corepack, and the npm tools of step 12 pull in the newest Node as their dependency.

use super::super::os;
use super::{run, stdout_of};
use crate::out::{self, info, ok, ok_if_changed, skip, warn};
use std::path::Path;

pub(super) fn step(_repo: &Path) {
    out::step("Node LTS (via mise)");
    os::brew_env();
    if !os::have("mise") {
        warn("mise not found — skipping node.");
        return;
    }
    if os::is_musl() {
        return;
    }
    // `mise ls node` marks the configured default with its alias (`lts`), and a version that is
    // configured but not installed with "(missing)". The npm tools' own Node (no alias) does not count.
    let default_lts = |l: &str| l.contains("lts") && !l.contains("missing");
    if stdout_of("mise", &["ls", "node"]).lines().any(default_lts) {
        skip("Node LTS (mise)");
    } else {
        info("Installing Node LTS via mise…");
        if run("mise", &["use", "-g", "node@lts"]) {
            ok("Node LTS installed");
        } else {
            warn("mise node install failed (network?) — re-run: ./dotfiles");
        }
    }
    ok_if_changed("node ready");
}
