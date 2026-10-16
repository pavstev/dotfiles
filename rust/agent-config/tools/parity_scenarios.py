#!/usr/bin/env python3
"""Differential harness, part 2: the paths the happy-path sweep does not reach.
Drift, promote, project mirroring, cli-config permissions, stale-entry removal.
Each side gets its own throwaway copy of the repo so `promote` cannot touch the real one."""
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

SRC = Path(os.environ.get("DOTFILES_REPO", Path(__file__).resolve().parents[3]))
PY = [sys.executable, str(SRC / "scripts" / "agent-config.py")]
RS = [str(SRC / "rust" / "agent-config" / "target" / "release" / "agent-config")]


def make_repo(dst):
    dst.mkdir(parents=True, exist_ok=True)
    for part in ("claude", "cursor", "scripts"):
        shutil.copytree(SRC / part, dst / part, symlinks=True,
                        ignore=shutil.ignore_patterns("__pycache__", "target"))
    return dst


def run(cmd, home, repo, argv):
    env = dict(os.environ)
    env.update(HOME=str(home), DOTFILES_REPO=str(repo), NO_COLOR="1",
               XDG_CONFIG_HOME=str(home / ".config"), PYTHONDONTWRITEBYTECODE="1")
    p = subprocess.run(cmd + argv, capture_output=True, text=True, env=env, cwd="/tmp")
    return p.returncode, p.stdout, p.stderr


def norm(t, home, repo):
    t = t.replace("/private" + str(repo), "~R~")  # macOS resolve() symlink
    t = t.replace(str(home), "~H~").replace(str(repo), "~R~")
    t = re.sub(r"acp2-(py|rs)-[A-Za-z0-9]+", "acp2-SIDE", t)
    return re.sub(r"\.bak-[0-9A-Za-z_-]+", ".bak-TS", t)


def tree(root, home, repo):
    out = {}
    for p in sorted(root.rglob("*")):
        rel = p.relative_to(root).as_posix()
        if rel.startswith(("Library/", "__pycache__", "scripts/__pycache__")) or "__pycache__" in rel:
            continue
        if p.is_symlink():
            out[rel] = "SYMLINK->" + norm(os.readlink(p), home, repo)
        elif p.is_dir():
            out[rel] = "DIR"
        else:
            try:
                out[rel] = norm(p.read_text(), home, repo)
            except Exception:
                out[rel] = "<bin %d>" % p.stat().st_size
    out = {re.sub(r"\.bak-[0-9A-Za-z_-]+", ".bak-TS", k): v for k, v in out.items()}
    return out


# name -> (home setup fn, [argv sequences to run in order])
def home_clean(h):
    (h / ".config" / "dotfiles").mkdir(parents=True, exist_ok=True)
    (h / ".config" / "dotfiles" / "settings").write_text('DOTFILES_PROFILE="shared"\n')


def home_drift_claude(h):
    """Claude edited the live settings.json after the last build."""
    home_clean(h)
    c = h / ".claude"
    c.mkdir(parents=True, exist_ok=True)
    (c / "settings.json").write_text(
        '{"permissions": {"allow": ["Bash(ls:*)", "Bash(rm:*)"]}, "model": "opus"}\n')
    (h / ".config" / "dotfiles" / "claude-settings.json").write_text(
        '{"permissions": {"allow": ["Bash(ls:*)"]}}\n')


def home_user_hooks(h):
    """Cursor's own hooks.json with a user hook plus one of our stale entries."""
    home_clean(h)
    c = h / ".cursor"
    c.mkdir(parents=True, exist_ok=True)
    (c / "hooks.json").write_text(json_dumps({
        "hooks": {
            "beforeSubmitPrompt": [{"command": "user-own-hook"}],
            "preToolUse": [{"matcher": "Task",
                            "hooks": [{"command": "python3 /OLD/PATH/guard_cursor.py tool",
                                       "cwd": "/OLD"}]}],
        }}))


def home_cli_config(h):
    """Cursor's own cli-config.json with permissions we did not manage."""
    home_clean(h)
    c = h / ".cursor"
    c.mkdir(parents=True, exist_ok=True)
    (c / "cli-config.json").write_text(
        '{"permissions": {"allow": ["WebFetch(domain:example.com)"]}}\n')


def home_opencode_project(h, proj):
    home_clean(h)
    p = proj / "src"
    p.mkdir(parents=True, exist_ok=True)
    (proj / ".git").mkdir(parents=True, exist_ok=True)
    oc = h / ".config" / "opencode"
    oc.mkdir(parents=True, exist_ok=True)
    (oc / "opencode.jsonc").write_text('{\n  "model": "mine"\n}\n')


def json_dumps(o):
    import json
    return json.dumps(o, indent=2) + "\n"


SCENARIOS = {
    "drift-build": (home_drift_claude, [["claude-settings", "build"],
                                        ["claude-settings", "build"]]),
    "stale-hooks": (home_user_hooks, [["cursor", "--profile", "shared"],
                                      ["cursor", "--profile", "shared"]]),
    "cli-config": (home_cli_config, [["cursor", "--profile", "shared"]]),
    "promote": (home_drift_claude, [["claude-settings", "build"],
                                    ["claude-settings", "promote", "permissions.allow"]]),
    "promote-missing-key": (home_drift_claude, [["claude-settings", "promote"]]),
    "project-mirror": (None, [["opencode", "--profile", "shared", "--projects", "PROJ"]]),
}


def run_side(kind, scen, setup, argvs):
    base = Path(tempfile.mkdtemp(prefix=f"acp2-{kind}-"))
    home = base / "home"
    home.mkdir()
    repo = make_repo(base / "repo")
    proj = base / "proj"
    # Python finds its repo from its own file location, so point it at the copy.
    cmd = ([sys.executable, str(repo / "scripts" / "agent-config.py")] if kind == "py"
           else [str(SRC / "rust" / "agent-config" / "target" / "release" / "agent-config")])
    if setup is None:
        home_opencode_project(home, proj)
    else:
        setup(home)
    rc = 0
    for a in argvs:
        a = [proj.as_posix() if x == "PROJ" else x for x in a]
        rc, out, err = run(cmd, home, repo, a)
    # repo mutations (promote) and home both matter
    return (norm(out, home, repo), norm(err, home, repo), rc,
            tree(home, home, repo), tree(repo, home, repo))


def main():
    fails = 0
    total = 0
    for scen, (setup, argvs) in SCENARIOS.items():
        total += 1
        o_p, e_p, rc_p, h_p, r_p = run_side("py", scen, setup, argvs)
        o_r, e_r, rc_r, h_r, r_r = run_side("rs", scen, setup, argvs)
        probs = []
        if rc_p != rc_r:
            probs.append(f"exit: py={rc_p} rs={rc_r}")
        if o_p != o_r:
            probs.append("stdout differs")
        if e_p != e_r:
            probs.append("stderr differs")
        for label, a, b in (("HOME", h_p, h_r), ("REPO", r_p, r_r)):
            if set(a) != set(b):
                probs.append(f"{label} file set: only_py={sorted(set(a)-set(b))[:8]} only_rs={sorted(set(b)-set(a))[:8]}")
            for k in sorted(set(a) & set(b)):
                if a[k] != b[k]:
                    probs.append(f"{label} content differs: {k}")
        if probs:
            fails += 1
            print(f"FAIL {scen}")
            for p in probs:
                print(f"     - {p}")
            if o_p != o_r:
                import difflib
                print("     " + "\n     ".join(list(difflib.unified_diff(
                    o_p.splitlines(), o_r.splitlines(), "py", "rs", lineterm="", n=0))[:30]))
            if e_p != e_r:
                print(f"     py err: {e_p[:400]!r}")
                print(f"     rs err: {e_r[:400]!r}")
        else:
            print(f"ok   {scen}")
    print(f"\n{total - fails}/{total} scenarios identical")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())