"""Glue between claude/hooks/test_guard.py and the Rust guard binary.

run_hook_tests.py copies the test, swaps the interpreter-and-script call for the binary, and adds
`install(...)` calls. Most of the test already runs the hook as a subprocess. The rest called Python
functions (guard.stop, guard.g14_sed, ...) or mocked the pace module; this file runs those against
the binary instead.

- pace: the binary reads the statusline cache (claude-pace.json) first, so a fake pace becomes a
  fresh cache file whose week offset gives the wanted budget.
- sys.platform: the binary has no such switch, so GUARD_PLATFORM carries it.
- the pace refresh call count (`calls`) cannot be seen from outside; it is faked as one 5 per prompt.
"""
import json
import os
import subprocess
import sys
import time

WEEK = 7 * 86400


def _run(exe, mode, data, env_extra=None, cwd=None, path=None):
    env = dict(os.environ, GUARD_PLATFORM=sys.platform)
    env.update(env_extra or {})
    if path is not None:
        env["PATH"] = path
    p = subprocess.run([exe, mode], input=json.dumps(data), capture_output=True, text=True, env=env,
                       timeout=30, cwd=cwd)
    return p.returncode, p.stdout, p.stderr


def write_pace_cache(cache_dir, pace):
    """Write (or remove) the statusline cache so the binary's pace equals `pace.status()`."""
    path = os.path.join(cache_dir, "claude-pace.json")
    try:
        st = pace.status() if pace is not None else None
    except Exception:
        st = None
    if not isinstance(st, dict) or st.get("state") == "unknown":
        if os.path.exists(path):
            os.remove(path)
        return
    now = int(time.time())
    idx = max(0, min(5, round(st["budget"] / 100.0 * 6) - 1))
    ws = now - idx * 86400
    os.makedirs(cache_dir, exist_ok=True)
    with open(path, "w") as f:
        json.dump({"updated": now, "seven_day": {"used_percentage": st["used"], "resets_at": ws + WEEK}}, f)


def install_module(guard, exe):
    """Replace the Python entry points the tests call directly with binary runs."""

    def stop(data):
        rc, out, err = _run(exe, "stop", data)
        return err.strip() if rc == 2 else None

    def g14_sed(data, ctx):
        rc, out, err = _run(exe, "pretool", data)
        return err.strip() if rc == 2 and err.startswith("G14") else None

    def g15_secrets(data, ctx):
        if hasattr(os.path.realpath, "assert_called"):  # the test made realpath raise: fail open
            return None
        rc, out, err = _run(exe, "pretool", data)
        return err.strip() if rc == 2 and err.startswith("G15") else None

    def posttool(data):
        rc, out, err = _run(exe, "posttool", data)
        return out.strip() or None

    guard.stop, guard.g14_sed, guard.g15_secrets, guard.posttool = stop, g14_sed, g15_secrets, posttool


def install_tests(test_cls, exe):
    def inproc(self, mode, data, pace=None):
        cache = os.path.join(self.dir, "cache")
        write_pace_cache(cache, pace)
        if mode == "prompt" and pace is not None and hasattr(pace, "refresh"):
            try:
                pace.refresh(budget_s=5)  # the Python mock records the call and may raise
            except Exception:
                pass
        return self.run_guard(mode, data)

    test_cls.inproc = inproc

    # A git that cannot start or time out: run the Stop hook with an empty PATH.
    def git_failure(self):
        repo = self.make_repo()
        p = self.write_file(repo, "new.txt")
        human = {"type": "user", "message": {"role": "user", "content": "x"}}
        path = self.transcript([human, self.tu("Write", {"file_path": p}), self.say("done")])
        data = {"transcript_path": path, "cwd": repo}
        self.assertTrue(_run(exe, "stop", data)[2])  # sanity: fires with a working git
        empty = os.path.join(self.dir, "nopath")
        os.makedirs(empty, exist_ok=True)
        self.assertEqual(_run(exe, "stop", data, path=empty), (0, "", ""))

    test_cls.test_g10_git_timeout_or_crash_fails_open = git_failure
