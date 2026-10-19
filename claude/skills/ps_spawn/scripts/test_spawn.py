#!/usr/bin/env python3
"""Tests for spawn.py. No tmux and no claude needed: they test the checks,
the command and the screen reading.

  python3 claude/skills/ps_spawn/scripts/test_spawn.py
"""
import datetime
import importlib.util
import json
import os
import shutil
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("spawn", os.path.join(HERE, "spawn.py"))
spawn = importlib.util.module_from_spec(spec)
spec.loader.exec_module(spawn)

OK_PANE = """ Claude Code v2.1.296
 ~/proj · /rc active
> Fix the login bug in app/auth.ts
  /remote-control is active · Continue here, on your phone, or at https://claude.ai/code/session_01XLjrBUwh9QVMBxLZ1bcqE4
"""


class Home:
    """A temp HOME with ~/.claude.json and ~/.claude/settings.json."""

    def __init__(self, trusted=True, bypass=False, logged_in=True, skip_setting=False):
        self.root = os.path.realpath(tempfile.mkdtemp(prefix="spawn-test-"))
        self.proj = os.path.join(self.root, "proj")
        os.makedirs(os.path.join(self.proj, "sub"))
        os.makedirs(os.path.join(self.root, ".claude"))
        g = {"projects": {self.proj: {"hasTrustDialogAccepted": trusted}}}
        if logged_in:
            g["oauthAccount"] = {"emailAddress": "x"}
        if bypass:
            g["bypassPermissionsModeAccepted"] = True
        with open(os.path.join(self.root, ".claude.json"), "w") as f:
            json.dump(g, f)
        with open(os.path.join(self.root, ".claude", "settings.json"), "w") as f:
            json.dump({"skipDangerousModePermissionPrompt": True} if skip_setting else {}, f)
        self.env = {"HOME": self.root}

    def done(self):
        shutil.rmtree(self.root, ignore_errors=True)


def which(name):
    return "/usr/bin/" + name


def check(home, mode="bypassPermissions", folder=None, name="n1", has=False, which_fn=which):
    return spawn.preflight(folder or home.proj, mode, name, home.env, which=which_fn,
                           tmux_has=lambda n: has)


class PreflightTest(unittest.TestCase):
    def test_all_good(self):
        h = Home(bypass=True)
        try:
            self.assertEqual(check(h), ("/usr/bin/claude", "/usr/bin/tmux"))
            self.assertEqual(check(h, folder=os.path.join(h.proj, "sub"))[0], "/usr/bin/claude")
        finally:
            h.done()

    def test_skip_setting_counts_as_accepted(self):
        h = Home(skip_setting=True)
        try:
            check(h)
        finally:
            h.done()

    def test_failures_name_the_fix(self):
        cases = [
            (dict(bypass=False), {}, "bypass warning"),
            (dict(trusted=False, bypass=True), {}, "not trusted"),
            (dict(logged_in=False, bypass=True), {}, "not logged in"),
            (dict(bypass=True), {"has": True}, "already exists"),
            (dict(bypass=True), {"which_fn": lambda n: None}, "tmux missing"),
            (dict(bypass=True), {"folder": "/nope/nowhere"}, "does not exist"),
        ]
        for home_kw, call_kw, want in cases:
            h = Home(**home_kw)
            try:
                with self.assertRaises(spawn.CheckFailed) as cm:
                    check(h, **call_kw)
                self.assertIn(want, str(cm.exception))
            finally:
                h.done()

    def test_other_modes_skip_bypass_check(self):
        h = Home(bypass=False)
        try:
            check(h, mode="acceptEdits")
        finally:
            h.done()


class CommandTest(unittest.TestCase):
    def test_clean_env(self):
        env = {"PATH": "/bin", "CLAUDECODE": "1", "CLAUDE_CODE_SESSION_ID": "x",
               "ANTHROPIC_API_KEY": "k", "MCP_X": "1", "CLAUDE_CONFIG_DIR": "/c", "HOME": "/h"}
        self.assertEqual(spawn.clean_env(env), {"PATH": "/bin", "CLAUDE_CONFIG_DIR": "/c", "HOME": "/h"})

    def test_names(self):
        now = datetime.datetime(2026, 10, 14, 9, 5)
        self.assertEqual(spawn.make_name("Fix the login bug!", now), "fix-the-login-0905")
        self.assertEqual(spawn.make_name("!!!", now), "session-0905")
        self.assertEqual(spawn.safe_name("a.b:c d"), "a-b-c-d")
        with self.assertRaises(spawn.CheckFailed):
            spawn.safe_name("...")

    def test_command_quotes_everything(self):
        cmd = spawn.shell_command("/x/claude", "n1", "bypassPermissions", "/tmp/it's here.md")
        self.assertIn("--remote-control n1 --name n1 --permission-mode bypassPermissions", cmd)
        self.assertIn("\"$(cat '/tmp/it'\"'\"'s here.md')\"", cmd)
        self.assertIn(spawn.EXIT_MARK, cmd)


class ClassifyTest(unittest.TestCase):
    P = "Fix the login bug in app/auth.ts"

    def test_ok(self):
        state, url = spawn.classify(OK_PANE, self.P, "/p")
        self.assertEqual(state, "ok")
        self.assertTrue(url.startswith("https://claude.ai/code/session_01XL"))

    def test_wait(self):
        self.assertEqual(spawn.classify("Claude Code v2\n> Fix the login", self.P, "/p")[0], "wait")
        self.assertEqual(spawn.classify("", self.P, "/p")[0], "wait")

    def test_dialogs(self):
        cases = [
            ("Do you trust the files in this folder?\n1. Yes", "not trusted"),
            ("WARNING: Claude Code running in Bypass Permissions mode\n1. No, exit", "bypass"),
            ("Select login method:\n1. Claude account", "not logged in"),
            ("error\n%s 1" % spawn.EXIT_MARK, "exited"),
        ]
        for pane, want in cases:
            state, why = spawn.classify(pane, self.P, "/p")
            self.assertEqual(state, "fail", pane)
            self.assertIn(want, why)

    def test_prompt_words_are_not_a_dialog(self):
        p = "Check why Bypass Permissions mode and trust the files in this folder fail"
        pane = "> " + p + "\n  working..."
        self.assertEqual(spawn.classify(pane, p, "/p")[0], "wait")


if __name__ == "__main__":
    unittest.main(verbosity=1)
