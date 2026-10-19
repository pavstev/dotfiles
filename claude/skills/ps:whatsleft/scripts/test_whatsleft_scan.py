#!/usr/bin/env python3
"""Tests for the check-freshness part of whatsleft-scan.py.

Small transcripts are written to a temp folder; no real sessions, no home paths.

  python3 claude/skills/ps:whatsleft/scripts/test_whatsleft_scan.py
"""
import contextlib
import importlib.util
import io
import json
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("whatsleft_scan", os.path.join(HERE, "whatsleft-scan.py"))
scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scan)

PASSED = "a check ran after the last edit, and it passed"
STALE = "files edited after the last check run"
FAILED = "the last check run failed"
RUNNING = "the last check is still running: wait for its notice, then read its exit line"


class Transcript:
    """Builds a small session transcript; every call is one tool use and its result."""

    def __init__(self, root):
        self.root = root
        self.lines = []
        self.n = 0

    def _ts(self):
        self.n += 1
        return "2026-10-14T10:%02d:00+00:00" % self.n

    def tool(self, name, inp, result="ok", error=False, cwd=None):
        tid = "toolu_%d" % self.n
        self.lines.append({"type": "assistant", "timestamp": self._ts(), "cwd": cwd or self.root,
                           "message": {"content": [{"type": "tool_use", "id": tid, "name": name, "input": inp}]}})
        self.lines.append({"type": "user", "timestamp": self._ts(),
                           "message": {"content": [{"type": "tool_result", "tool_use_id": tid,
                                                    "content": result, "is_error": error}]}})
        return tid

    def edit(self, rel="app.py"):
        return self.tool("Edit", {"file_path": os.path.join(self.root, rel)})

    def bash(self, cmd, **kw):
        return self.tool("Bash", {"command": cmd}, **kw)

    def bash_bg(self, cmd, bg_id="bg1"):
        return self.tool("Bash", {"command": cmd, "run_in_background": True},
                         result="Command running in background with ID: %s. Output is being written to: "
                                "/x/%s.output" % (bg_id, bg_id))

    def notice(self, tool_id, status, summary, task="bg1"):
        text = ("<task-notification>\n<task-id>%s</task-id>\n<tool-use-id>%s</tool-use-id>\n"
                "<status>%s</status>\n<summary>%s</summary>\n</task-notification>"
                % (task, tool_id, status, summary))
        self.lines.append({"type": "attachment", "timestamp": self._ts(),
                           "attachment": {"commandMode": "task-notification", "prompt": text}})

    def freshness(self):
        fd, path = tempfile.mkstemp(suffix=".jsonl", dir=self.root)
        with os.fdopen(fd, "w") as f:
            for obj in self.lines:
                f.write(json.dumps(obj) + "\n")
        s = scan.read_session(path)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            scan.check_freshness(s, self.root)
        return buf.getvalue()


class FreshnessTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = os.path.realpath(self._tmp.name)
        self.t = Transcript(self.root)

    def tearDown(self):
        self._tmp.cleanup()

    # --- no false alarm ---
    def test_foreground_unmasked_check_after_edit_passes(self):
        self.t.edit()
        self.t.bash("pnpm check")
        out = self.t.freshness()
        self.assertIn("OK", out)
        self.assertIn(PASSED, out)
        self.assertNotIn("ME", out)

    def test_foreground_pipe_with_pipefail_passes(self):
        self.t.edit()
        self.t.bash("set -o pipefail; pnpm check 2>&1 | tail -20")
        self.assertIn(PASSED, self.t.freshness())

    def test_check_chained_with_and_is_not_masked(self):
        self.t.edit()
        self.t.bash("pnpm check && echo done")
        self.assertIn(PASSED, self.t.freshness())

    def test_failed_foreground_check_still_fails(self):
        self.t.edit()
        self.t.bash("pnpm check", error=True)
        self.assertIn(FAILED, self.t.freshness())

    # --- a background check counts by its notice ---
    def test_background_check_without_notice_is_still_running(self):
        self.t.edit()
        self.t.bash_bg("pnpm check")
        out = self.t.freshness()
        self.assertIn("ME", out)
        self.assertIn(RUNNING, out)
        self.assertNotIn(PASSED, out)

    def test_background_check_completed_exit_0_passed(self):
        self.t.edit()
        tid = self.t.bash_bg("pnpm check")
        self.t.notice(tid, "completed", 'Background command "pnpm check" completed (exit code 0)')
        self.assertIn(PASSED, self.t.freshness())

    def test_background_check_failed_or_non_zero_exit_failed(self):
        for status, summary in (("failed", 'Background command "pnpm check" failed with exit code 1'),
                                ("completed", 'Background command "pnpm check" completed (exit code 2)'),
                                ("killed", 'Background command "pnpm check" was stopped')):
            with self.subTest(status=status, summary=summary):
                t = Transcript(self.root)
                t.edit()
                tid = t.bash_bg("pnpm check")
                t.notice(tid, status, summary)
                out = t.freshness()
                self.assertIn(FAILED, out)
                self.assertNotIn(PASSED, out)

    # --- a masked exit code ---
    def test_masked_exit_code_is_named(self):
        cases = {
            "pnpm check; echo": "; echo",
            "pnpm check; echo done": "; echo",
            "pnpm check || true": "|| true",
            "pnpm check; true": "; true",
            "pnpm check 2>&1 | tail -20": "| tail",
            "pnpm check | tee check.log": "| tee",
            "pnpm check | head -5": "| head",
            "cd web && pnpm check 2>&1 | tail": "| tail",
        }
        for cmd, mask in cases.items():
            with self.subTest(cmd=cmd):
                t = Transcript(self.root)
                t.edit()
                t.bash(cmd)
                out = t.freshness()
                self.assertIn("ME", out)
                self.assertIn("the last check's exit code is hidden (%s): read its log or run it again "
                              "without it" % mask, out)
                self.assertNotIn(PASSED, out)

    def test_masked_background_check_with_exit_0_notice_is_hidden(self):
        self.t.edit()
        tid = self.t.bash_bg("pnpm check; echo")
        self.t.notice(tid, "completed", 'Background command "pnpm check; echo" completed (exit code 0)')
        out = self.t.freshness()
        self.assertIn("exit code is hidden (; echo)", out)
        self.assertNotIn(PASSED, out)

    # --- a Bash in-place edit counts as an edit ---
    def test_bash_in_place_edit_counts(self):
        cmds = ["sed -i 's/a/b/' app.py", "sed -i.bak 's/a/b/' app.py", "sed -E -i 's/a/b/' app.py",
                "sed --in-place 's/a/b/' app.py", "perl -pi -e 's/a/b/' app.py", "perl -i -pe 's/a/b/' app.py",
                "perl -0pi -e 's/a/b/' app.py", "perl -pi.bak -e 's/a/b/' app.py"]
        for cmd in cmds:
            with self.subTest(cmd=cmd):
                t = Transcript(self.root)
                t.edit()
                t.bash("pnpm check")
                t.bash(cmd)
                out = t.freshness()
                self.assertIn(STALE, out)
                self.assertNotIn(PASSED, out)

    def test_bash_reads_are_not_edits(self):
        cmds = ["sed -n 1,5p app.py", "grep -i todo app.py", "sed -n 1p app.py | grep -i x",
                "perl -MList::Util -e 'print 1'", "perl -ne 'print if /x/' app.py"]
        for cmd in cmds:
            with self.subTest(cmd=cmd):
                t = Transcript(self.root)
                t.edit()
                t.bash("pnpm check")
                t.bash(cmd)
                self.assertIn(PASSED, t.freshness())

    def test_in_place_edit_outside_the_repo_is_not_an_edit(self):
        other = os.path.join(os.path.dirname(self.root), "elsewhere")
        self.t.edit()
        self.t.bash("pnpm check")
        self.t.bash("sed -i 's/a/b/' notes.txt", cwd=other)
        self.assertIn(PASSED, self.t.freshness())

    def test_in_place_edit_without_any_check(self):
        self.t.bash("sed -i 's/a/b/' app.py")
        self.assertIn("no test/lint/check ran", self.t.freshness())

    # --- the 2026-10-14 session ---
    def test_replay_2026_10_14(self):
        t = self.t
        t.edit()
        t.bash("sed -i 's/old/new/' app.py")
        first = t.bash_bg("pnpm check; echo", bg_id="bg1")
        t.notice(first, "completed", 'Background command "pnpm check; echo" completed (exit code 0)', task="bg1")
        t.bash_bg("pnpm check", bg_id="bg2")  # no notice yet
        out = t.freshness()
        self.assertIn(RUNNING, out)
        self.assertNotIn(PASSED, out)


if __name__ == "__main__":
    unittest.main(verbosity=1)
