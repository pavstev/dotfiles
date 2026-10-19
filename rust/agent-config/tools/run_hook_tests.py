#!/usr/bin/env python3
"""Run the Python hook tests against the Rust binaries.

  cargo build --release --manifest-path rust/agent-config/Cargo.toml
  python3 rust/agent-config/tools/run_hook_tests.py

The reference tests start the Python hook as a subprocess. This copies each test into a temp
folder, swaps the interpreter-and-script pair for the Rust binary, and runs the copy. The tests that
called Python functions or mocked the pace module are redirected to the binary by hook_test_shim.py.
"""
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
REL = os.path.join(REPO, "rust", "agent-config", "target", "release")

# (test file, hook dir, binary, pattern of the argv list to replace, tail of the new list)
CASES = [
    ("claude/hooks/test_guard.py", "claude/hooks", "guard", r"\[sys\.executable, GUARD,", ","),
    ("cursor/hooks/test_guard_cursor.py", "cursor/hooks", "guard_cursor", r"\[sys\.executable, script, mode\]", ", mode]"),
]


SHIM_MODULE = "import guard  # noqa: E402"
SHIM_MAIN = 'if __name__ == "__main__":'


def add_shim(src, exe):
    """Run the direct-call and pace-mock tests of test_guard.py against the binary."""
    head = (
        f"{SHIM_MODULE}\nsys.path.insert(0, {HERE!r})\nimport hook_test_shim as _shim\n"
        f"_shim.install_module(guard, {exe!r})\n"
    )
    tail = f"_shim.install_tests(GuardTest, {exe!r})\n\n\n{SHIM_MAIN}"
    if SHIM_MODULE not in src or SHIM_MAIN not in src:
        return None
    return src.replace(SHIM_MODULE, head, 1).replace(SHIM_MAIN, tail, 1)


def main():
    failed = 0
    for test, hook_dir, binary, pattern, tail in CASES:
        exe = os.path.join(REL, binary + (".exe" if sys.platform == "win32" else ""))
        if not os.path.exists(exe):
            print(f"missing {exe}: run cargo build --release first")
            return 2
        src = open(os.path.join(REPO, test), encoding="utf-8").read()
        repl = f"[{exe!r}" + tail
        out, n = re.subn(pattern, lambda _m: repl, src)
        # The test finds its hook dir from __file__; point it at the repo copy instead.
        out = re.sub(r"^HERE = .*$", f"HERE = {os.path.join(REPO, hook_dir)!r}", out, count=1, flags=re.M)
        if n == 0:
            print(f"{test}: nothing to replace, the test changed shape")
            return 2
        if binary == "guard":
            out = add_shim(out, exe)
            if out is None:
                print(f"{test}: shim anchors not found, the test changed shape")
                return 2
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, os.path.basename(test))
            with open(path, "w", encoding="utf-8") as f:
                f.write(out)
            print(f"== {test} against {binary} ({n} call sites)")
            failed |= subprocess.run([sys.executable, "-I", path]).returncode != 0
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
