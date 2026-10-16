#!/usr/bin/env python3
"""Differential harness: run the Python and Rust agent-config on identical
fixtures and compare exit code, stdout, and the resulting HOME tree."""
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(os.environ.get("DOTFILES_REPO", Path(__file__).resolve().parents[3]))
PY = [sys.executable, str(REPO / "scripts" / "agent-config.py")]
RS = [str(REPO / "rust" / "agent-config" / "target" / "release" / "agent-config")]

CASES = [
    ["user-rules"],
    ["user-rules", "--profile", "owner"],
    ["user-rules", "--profile", "shared"],
    ["cursor", "--profile", "shared"],
    ["cursor", "--profile", "shared"],  # second run: idempotency / skip path
    ["cursor", "--profile", "owner"],
    ["cursor", "--profile", "shared", "--dry-run"],
    ["opencode", "--profile", "shared"],
    ["opencode", "--profile", "shared"],
    ["opencode", "--profile", "owner"],
    ["opencode", "--profile", "shared", "--dry-run"],
    ["claude-settings", "build"],
    ["claude-settings", "build"],
    ["--help"],
]

VERBOSE = "-v" in sys.argv


def run(cmd, home, argv):
    env = dict(os.environ)
    env["HOME"] = str(home)
    env["DOTFILES_REPO"] = str(REPO)
    env["NO_COLOR"] = "1"
    env["XDG_CONFIG_HOME"] = str(home / ".config")
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env.pop("DOTFILES_PROFILE", None)
    p = subprocess.run(cmd + argv, capture_output=True, text=True, env=env, cwd="/tmp")
    return p.returncode, p.stdout, p.stderr


def normalize(text, home):
    t = text.replace(str(home), "~HOME~")
    t = re.sub(r"\.bak-[0-9A-Za-z_-]+", ".bak-TS", t)
    t = re.sub(r"\d{4}-\d{2}-\d{2}", "DATE", t)
    return t


# Noise that is not a behavioural divergence.
IGNORE_PREFIX = ("Library/", "Caches/")


def tree(home):
    out = {}
    for p in sorted(home.rglob("*")):
        rel = p.relative_to(home)
        if rel.as_posix().startswith(IGNORE_PREFIX):
            continue
        if p.is_symlink():
            out[str(rel)] = "SYMLINK->" + os.readlink(p).replace(str(home), "~HOME~")
        elif p.is_dir():
            out[str(rel)] = "DIR"
        else:
            try:
                body = normalize(p.read_text(), home)
            except Exception:
                body = "<binary %d bytes>" % p.stat().st_size
            key = re.sub(r"\.bak-[0-9A-Za-z_-]+", ".bak-TS", str(rel))
            out[key] = body
    return out


def seed(home, profile=None):
    """Mimic a real machine: private layer + a user-owned edit in Cursor's own JSON."""
    conf = home / ".config" / "dotfiles"
    conf.mkdir(parents=True, exist_ok=True)
    (conf / "settings").write_text(f'DOTFILES_PROFILE="{profile}"\n' if profile else "")
    if profile == "owner":
        (conf / "owner.md").write_text("<private owner rules>\n")
    (conf / "claude-settings.json").write_text('{"permissions": {"allow": ["Bash(ls:*)"]}}\n')
    cur = home / ".cursor"
    cur.mkdir(parents=True, exist_ok=True)
    (cur / "hooks.json").write_text(
        '{"hooks": {"beforeSubmitPrompt": [{"command": "user-own-hook"}]}}\n'
    )
    oc = home / ".config" / "opencode"
    oc.mkdir(parents=True, exist_ok=True)
    (oc / "opencode.jsonc").write_text('{\n  // user comment\n  "model": "mine"\n}\n')


def main():
    args = [a for a in sys.argv[1:] if a != "-v"]
    only = args[0] if args else None
    fails = 0
    total = 0
    for profile in (None, "shared", "owner"):
        base = Path(tempfile.mkdtemp(prefix="acp-"))
        homes = {}
        for kind in ("py", "rs"):
            h = base / kind / "home"
            h.mkdir(parents=True)
            seed(h, profile)
            homes[kind] = h

        for argv in CASES:
            if only and only not in " ".join(argv):
                continue
            total += 1
            rc_p, out_p, err_p = run(PY, homes["py"], argv)
            rc_r, out_r, err_r = run(RS, homes["rs"], argv)
            problems = []
            if rc_p != rc_r:
                problems.append(f"exit: py={rc_p} rs={rc_r}")
            np_, nr_ = normalize(out_p, homes["py"]), normalize(out_r, homes["rs"])
            if np_ != nr_:
                problems.append("stdout differs")
            tp, tr = tree(homes["py"]), tree(homes["rs"])
            if set(tp) != set(tr):
                only_p, only_r = set(tp) - set(tr), set(tr) - set(tp)
                problems.append(f"file set: only_py={sorted(only_p)[:6]} only_rs={sorted(only_r)[:6]}")
            for k in sorted(set(tp) & set(tr)):
                if tp[k] != tr[k]:
                    problems.append(f"content differs: {k}")
                    if VERBOSE:
                        import difflib
                        d = list(difflib.unified_diff(
                            tp[k].splitlines(), tr[k].splitlines(),
                            "py", "rs", lineterm="", n=1))
                        problems.append("      " + "\n      ".join(d[:24]))
            tag = f"[profile={profile}] agent-config {' '.join(argv)}"
            if problems:
                fails += 1
                print(f"FAIL {tag}")
                for p in problems:
                    print(f"     - {p}")
                if np_ != nr_:
                    print(f"     py stdout: {np_[:600]!r}")
                    print(f"     rs stdout: {nr_[:600]!r}")
            else:
                print(f"ok   {tag}")
        shutil.rmtree(base, ignore_errors=True)

    print(f"\n{total - fails}/{total} cases identical")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())