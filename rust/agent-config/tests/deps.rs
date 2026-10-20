//! Gate for the agent tools: no crate in the build may compile C code, on any target we ship.
//! Also checks that the `ctx` and `sessions` bins exist.

use std::process::Command;

const TARGETS: [&str; 3] = [
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-gnu",
];

#[test]
fn no_c_build_on_any_target() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    for target in TARGETS {
        let out = Command::new(&cargo)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args([
                "tree",
                "--target",
                target,
                "-e",
                "normal,build",
                "--prefix",
                "none",
                "--format",
                "{p}",
            ])
            .output()
            .expect("run cargo tree");
        assert!(
            out.status.success(),
            "cargo tree failed for {target}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            for bad in ["cc ", "cmake ", "bindgen "] {
                assert!(
                    !line.starts_with(bad),
                    "C build crate `{line}` in the tree for {target}; find it with `cargo tree -i <crate> --target {target}`"
                );
            }
        }
    }
}

#[test]
fn stub_bins_exist() {
    for exe in [env!("CARGO_BIN_EXE_ctx"), env!("CARGO_BIN_EXE_sessions")] {
        let status = Command::new(exe)
            .arg("--help")
            .status()
            .expect("run stub bin");
        assert!(status.success(), "{exe} --help must exit 0");
    }
}
