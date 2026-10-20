//! 50-node-pnpm: Node LTS + corepack + pnpm via mise. Installs only what's missing. Tolerant: a
//! failed network step warns and goes on. Alpine skips it: Node from mise needs glibc libraries
//! or a compiler there.

use super::super::os;
use super::{quiet, run, stdout_of};
use crate::out::{self, info, ok, ok_if_changed, skip, warn};
use std::path::Path;

pub fn step(_repo: &Path) {
    out::step("Node + pnpm (via mise)");
    os::brew_env();
    if !os::have("mise") {
        warn("mise not found — skipping node/pnpm.");
        return;
    }
    if os::is_musl() { return; }

    if stdout_of("mise", &["ls", "node"]).lines().any(|l| !l.is_empty()) {
        skip("Node (mise)");
    } else {
        info("Installing Node LTS via mise…");
        if !run("mise", &["use", "-g", "node@lts"]) {
            warn("mise node install failed (network?) — re-run: ./dotfiles");
            return;
        }
        ok("Node LTS installed");
    }

    quiet("mise", &["exec", "--", "corepack", "enable"]);
    if quiet("mise", &["exec", "--", "pnpm", "--version"]) {
        skip("pnpm");
    } else {
        info("Activating pnpm via corepack…");
        if !quiet("mise", &["exec", "--", "corepack", "prepare", "pnpm@latest", "--activate"]) {
            warn("pnpm activation failed — re-run: ./dotfiles");
        }
    }
    ok_if_changed("node + pnpm ready");
}
