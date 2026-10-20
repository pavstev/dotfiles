#!/usr/bin/env python3
"""spawn.py: start a new Claude Code session with Remote Control on, in a detached tmux.

One start, no retries. Every check that could stop the new session (no tmux,
untrusted folder, bypass warning never accepted, name taken) runs BEFORE the
start. After the start it waits once for the Remote Control link on screen.

  python3 spawn.py --prompt-file p.md                  # folder: current, mode: bypassPermissions
  python3 spawn.py --prompt-file p.md --dir ~/proj --name fix-login
  python3 spawn.py --prompt-file p.md --mode acceptEdits
  python3 spawn.py --prompt-file p.md --dry-run        # checks + the command, no start

Exit: 0 started · 2 bad args · 3 a check failed (FIX line says what to do)
      4 the session stopped or showed a dialog · 5 no Remote Control link in time.
Prints: NAME, MODE, URL, WATCH and STOP lines. Stdlib only, Python 3.9+.
"""
import argparse
import datetime
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import time

SOCKET = "claude-spawn"  # own tmux server, started from a clean env
MODES = ("bypassPermissions", "acceptEdits", "auto", "default", "dontAsk", "plan")
URL_RE = re.compile(r"https://claude\.ai/code/session_[A-Za-z0-9]+")
EXIT_MARK = "CLAUDE_EXITED"
# Env of the parent session that must not leak into the child.
DROP_ENV = re.compile(r"^(CLAUDE(?!_CONFIG_DIR$).*|ANTHROPIC_.*|MCP_.*)$")
DIALOGS = (
    (re.compile(r"trust (the files|this folder)", re.I),
     "folder not trusted: run `claude` once in %(dir)s and accept"),
    (re.compile(r"Bypass Permissions mode", re.I),
     "bypass warning not accepted: run `claude --permission-mode bypassPermissions` once in a terminal and accept"),
    (re.compile(r"Select login method", re.I),
     "not logged in to claude.ai: run `claude` and /login"),
)


class CheckFailed(Exception):
    pass


def config_paths(env):
    cfg = env.get("CLAUDE_CONFIG_DIR") or os.path.join(env.get("HOME", os.path.expanduser("~")), ".claude")
    if env.get("CLAUDE_CONFIG_DIR"):
        global_json = os.path.join(cfg, ".claude.json")
    else:
        global_json = os.path.join(os.path.dirname(cfg), ".claude.json")
    return cfg, global_json


def load_json(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def clean_env(env):
    return {k: v for k, v in env.items() if not DROP_ENV.match(k)}


def make_name(prompt, now=None):
    words = re.findall(r"[A-Za-z0-9]+", prompt.lower())[:3] or ["session"]
    stamp = (now or datetime.datetime.now()).strftime("%H%M")
    return "-".join(words + [stamp])[:40]


def safe_name(name):
    out = re.sub(r"[^A-Za-z0-9_-]+", "-", name).strip("-")[:40]
    if not out:
        raise CheckFailed("name is empty after cleaning: use letters, digits, - or _")
    return out


def trusted(folder, projects):
    """True when the folder or a parent has hasTrustDialogAccepted."""
    p = os.path.realpath(folder)
    while True:
        if (projects.get(p) or {}).get("hasTrustDialogAccepted"):
            return True
        parent = os.path.dirname(p)
        if parent == p:
            return False
        p = parent


def preflight(folder, mode, name, env, which=shutil.which, tmux_has=None):
    """Return (claude_path, tmux_path). Raise CheckFailed with the fix."""
    tmux = which("tmux")
    if not tmux:
        raise CheckFailed("tmux missing: run `dots`")
    claude = which("claude")
    if not claude:
        raise CheckFailed("claude missing on PATH: install Claude Code")
    if not os.path.isdir(folder):
        raise CheckFailed("folder does not exist: %s" % folder)
    cfg, global_json = config_paths(env)
    g = load_json(global_json)
    if not g.get("oauthAccount"):
        raise CheckFailed("not logged in to claude.ai (Remote Control needs it): run `claude` and /login")
    if not trusted(folder, g.get("projects") or {}):
        raise CheckFailed("folder not trusted: run `claude` once in %s and accept" % folder)
    if mode == "bypassPermissions":
        settings = load_json(os.path.join(cfg, "settings.json"))
        if not (g.get("bypassPermissionsModeAccepted") or settings.get("skipDangerousModePermissionPrompt")):
            raise CheckFailed("bypass warning never accepted on this machine: run "
                              "`claude --permission-mode bypassPermissions` once in a terminal, accept, "
                              "then /exit. Or pass --mode acceptEdits.")
    if tmux_has is None:
        tmux_has = lambda n: subprocess.run(  # noqa: E731
            [tmux, "-L", SOCKET, "has-session", "-t", "=" + n],
            capture_output=True).returncode == 0
    if tmux_has(name):
        raise CheckFailed("tmux session %s already exists: pick another --name" % name)
    return claude, tmux


def shell_command(claude, name, mode, prompt_file):
    """The command tmux runs. The prompt is read from a file, so no quoting issues."""
    return ("%s --remote-control %s --name %s --permission-mode %s \"$(cat %s)\"; "
            "echo %s $?; exec sleep 86400") % (
        shlex.quote(claude), shlex.quote(name), shlex.quote(name), shlex.quote(mode),
        shlex.quote(prompt_file), EXIT_MARK)


def classify(pane, prompt, folder):
    """Return ('ok', url) | ('fail', why) | ('wait', '')."""
    if EXIT_MARK in pane:
        return "fail", "claude exited at start"
    flat = " ".join(pane.split())
    first = " ".join(prompt.split())[:20]
    sent = bool(first) and first in flat
    if not sent and not URL_RE.search(pane):  # dialogs show before the prompt is sent; after that, the text may be the prompt's
        for rx, why in DIALOGS:
            if rx.search(pane):
                return "fail", why % {"dir": folder}
    m = URL_RE.search(pane)
    if m:  # the link shows only once the session runs; a long prompt may show as "[Pasted text]"
        return "ok", m.group(0)
    return "wait", ""


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--prompt-file", help="file with the prompt for the new session")
    src.add_argument("--prompt", help="the prompt itself (prefer --prompt-file)")
    ap.add_argument("--dir", default=os.getcwd(), help="folder for the new session (default: current)")
    ap.add_argument("--name", help="session name on the phone and in tmux (default: from the prompt)")
    ap.add_argument("--mode", default="bypassPermissions", choices=MODES, help="permission mode")
    ap.add_argument("--wait", type=int, default=60, help="seconds to wait for the Remote Control link")
    ap.add_argument("--dry-run", action="store_true", help="run the checks, print the command, start nothing")
    a = ap.parse_args(argv)

    if a.prompt_file:
        try:
            with open(os.path.expanduser(a.prompt_file), encoding="utf-8") as f:
                prompt = f.read()
        except OSError as e:
            print("spawn: cannot read the prompt file: %s" % e)
            return 2
    else:
        prompt = a.prompt
    if not prompt.strip():
        print("spawn: the prompt is empty")
        return 2
    folder = os.path.realpath(os.path.expanduser(a.dir))
    env = clean_env(os.environ)
    try:
        name = safe_name(a.name or make_name(prompt))
        claude, tmux = preflight(folder, a.mode, name, os.environ)
    except CheckFailed as e:
        print("FAIL  %s" % e)
        print("FIX   %s" % str(e).split(": ", 1)[-1])
        return 3

    cfg, _ = config_paths(os.environ)
    keep = os.path.join(cfg, "spawn")
    os.makedirs(keep, exist_ok=True)
    prompt_file = os.path.join(keep, name + ".md")
    cmd = shell_command(claude, name, a.mode, prompt_file)
    tmux_cmd = [tmux, "-L", SOCKET, "new-session", "-d", "-s", name, "-c", folder,
                "-x", "200", "-y", "50", cmd]
    if a.dry_run:
        print("CHECKS ok")
        print("CMD   " + " ".join(shlex.quote(x) for x in tmux_cmd))
        return 0
    with open(prompt_file, "w", encoding="utf-8") as f:
        f.write(prompt)

    r = subprocess.run(tmux_cmd, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        print("FAIL  tmux did not start: %s" % (r.stderr.strip() or r.returncode))
        return 4
    deadline = time.time() + a.wait
    state, info, pane = "wait", "", ""
    while time.time() < deadline:
        time.sleep(1)
        pane = subprocess.run([tmux, "-L", SOCKET, "capture-pane", "-p", "-t", name],
                              env=env, capture_output=True, text=True).stdout
        state, info = classify(pane, prompt, folder)
        if state != "wait":
            break
    watch = "tmux -L %s attach -t %s" % (SOCKET, name)
    stop = "tmux -L %s kill-session -t %s" % (SOCKET, name)
    if state == "ok":
        print("NAME  %s" % name)
        print("MODE  %s in %s" % (a.mode, folder))
        print("URL   %s" % info)
        print("WATCH phone: Claude app > Code > %s · Terminal: %s" % (name, watch))
        print("STOP  %s" % stop)
        return 0
    tail = "\n".join([ln for ln in pane.splitlines() if ln.strip()][-12:])
    if state == "fail":
        subprocess.run([tmux, "-L", SOCKET, "kill-session", "-t", name], env=env, capture_output=True)
        print("FAIL  %s (session stopped)" % info)
        print(tail)
        return 4
    print("FAIL  no Remote Control link after %ds. The session still runs." % a.wait)
    print("WATCH %s" % watch)
    print("STOP  %s" % stop)
    print(tail)
    return 5


if __name__ == "__main__":
    sys.exit(main())
