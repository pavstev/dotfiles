#!/usr/bin/env python3
"""Tests for route.py and for the links between ps:whatsleft, ps:askme and ps:mic-drop.

  python3 claude/skills/ps:whatsleft/scripts/test_route.py
"""
import importlib.util
import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
SKILLS = os.path.dirname(os.path.dirname(HERE))
spec = importlib.util.spec_from_file_location("route", os.path.join(HERE, "route.py"))
route = importlib.util.module_from_spec(spec)
spec.loader.exec_module(route)


def routes(res):
    return {x["id"]: x["route"] for x in res["routes"]}


# The test scenario from SOURCES.md: CSV export, tests, deploy, env var in AWS.
SCENARIO = [
    {"id": "m1", "kind": "work", "text": "No test for the export"},
    {"id": "m2", "kind": "work", "text": "Lint and tests after the last edit", "after": ["m1"]},
    {"id": "m3", "kind": "work", "risk": "prod-config", "text": "Set REPORTS_BUCKET in Production"},
    {"id": "m4", "kind": "work", "risk": "push", "asked": True, "text": "Push and deploy",
     "after": ["m2", "m3"]},
    {"id": "y1", "kind": "no-tool", "tried": "Chrome: AWS sign-in page", "text": "Sign in to AWS"},
    {"id": "o1", "kind": "optional", "text": "CSV export on invoices too"},
    {"id": "o2", "kind": "optional", "text": "README line for the export"},
    {"id": "o3", "kind": "optional", "who": "you", "text": "Speed Insights banner"},
    {"id": "t1", "kind": "task", "stop": True, "text": "dev server, idle 90m"},
    {"id": "t2", "kind": "task", "text": "test watcher, busy"},
    {"id": "d0", "kind": "work", "done": True, "text": "CSV export code"},
]


class RouteTest(unittest.TestCase):
    def test_scenario(self):
        res = route.route(SCENARIO)
        r = routes(res)
        self.assertEqual(r["m1"], "DO")
        self.assertEqual(r["m2"], "WAIT")       # waits on m1
        self.assertEqual(r["m3"], "ASK")        # risky, not asked
        self.assertEqual(r["m4"], "WAIT")       # asked, but waits on m2 and m3
        self.assertEqual(r["y1"], "YOU")
        self.assertEqual(r["o3"], "YOU")        # optional, only the user can do it
        self.assertEqual(r["t2"], "KEEP")
        self.assertEqual(r["d0"], "DONE")
        # costly first, then the task, then optional rows; cap 4
        self.assertEqual(res["ask"], ["m3", "t1", "o1", "o2"])
        self.assertEqual(res["default"], [])
        self.assertEqual(res["then"], "rerun")

    def test_cap_defaults_the_cheapest(self):
        rows = [{"id": "o%d" % i, "kind": "optional", "text": "idea %d" % i} for i in range(3)]
        rows.insert(1, {"id": "d1", "kind": "decision", "text": "merge or PR"})
        res = route.route(rows, cap=2)
        self.assertEqual(res["ask"], ["d1", "o0"])
        self.assertEqual(res["default"], ["o1", "o2"])
        self.assertEqual(routes(res)["o2"], "DEFAULT")

    def test_kill_makes_stop_tasks_do(self):
        rows = [{"id": "t1", "kind": "task", "stop": True, "text": "idle watcher"}]
        self.assertEqual(routes(route.route(rows))["t1"], "ASK")
        self.assertEqual(routes(route.route(rows, kill=True))["t1"], "DO")

    def test_nothing_to_ask_means_mic_drop(self):
        rows = [{"id": "m1", "kind": "work", "text": "run lint"},
                {"id": "y1", "kind": "login", "text": "gh auth login"}]
        res = route.route(rows)
        self.assertEqual(res["ask"], [])
        self.assertEqual(res["then"], "mic-drop")
        self.assertEqual(route.route([])["then"], "mic-drop")

    def test_wait_on_do_reruns(self):
        rows = [{"id": "a", "kind": "work", "text": "build"},
                {"id": "b", "kind": "work", "text": "test", "after": ["a"]}]
        self.assertEqual(route.route(rows)["then"], "rerun")

    def test_answers_unlock(self):
        # second round: the user said yes to m3, m1 and m2 are done
        rows = json.loads(json.dumps(SCENARIO))
        for r in rows:
            if r["id"] in ("m1", "m2"):
                r["done"] = True
            if r["id"] == "m3":
                r["asked"] = True
            if r["id"] in ("o1", "o2", "t1"):
                r["done"] = True
        res = route.route(rows)
        r = routes(res)
        self.assertEqual(r["m3"], "DO")
        self.assertEqual(r["m4"], "WAIT")
        self.assertEqual(res["then"], "rerun")

    def test_dependent_question_waits(self):
        rows = [{"id": "v1", "kind": "value", "text": "bucket name?"},
                {"id": "e1", "kind": "work", "risk": "prod-config", "text": "set it in Production",
                 "after": ["v1"]}]
        res = route.route(rows)
        self.assertEqual(res["ask"], ["v1"])
        self.assertEqual(routes(res)["e1"], "WAIT")
        rows[0]["done"] = True
        self.assertEqual(route.route(rows)["ask"], ["e1"])

    def test_wait_chain_is_order_free(self):
        # c waits on b, b waits on a: only a is asked, in any row order
        a = {"id": "a", "kind": "decision", "text": "merge or PR"}
        b = {"id": "b", "kind": "decision", "text": "branch name", "after": ["a"]}
        c = {"id": "c", "kind": "decision", "text": "PR title", "after": ["b"]}
        for rows in ([a, b, c], [c, b, a], [b, c, a]):
            res = route.route([dict(r) for r in rows])
            self.assertEqual(res["ask"], ["a"])
            self.assertEqual(routes(res), {"a": "ASK", "b": "WAIT", "c": "WAIT"})
        # a DO row behind a waiting DO row waits too
        rows = [{"id": "d2", "kind": "work", "text": "deploy", "after": ["d1"]},
                {"id": "d1", "kind": "work", "text": "test", "after": ["q"]},
                {"id": "q", "kind": "decision", "text": "which env"}]
        self.assertEqual(routes(route.route(rows)), {"d2": "WAIT", "d1": "WAIT", "q": "ASK"})

    def test_cycle_is_bad_input(self):
        rows = [{"id": "a", "kind": "work", "text": "t", "after": ["b"]},
                {"id": "b", "kind": "work", "text": "u", "after": ["a"]}]
        with self.assertRaises(route.BadInput):
            route.route(rows)
        with self.assertRaises(route.BadInput):
            route.route([{"id": "a", "kind": "work", "text": "t", "after": [["b"]]}])

    def test_semicolon_in_text_keeps_the_handoff_line(self):
        rows = [{"id": "d1", "kind": "decision", "text": "keep a; or drop b"},
                {"id": "d2", "kind": "decision", "text": "merge or PR"}]
        line = [l for l in route.render(route.route(rows)).splitlines() if "whatsleft:" in l][0]
        self.assertEqual(line.split("whatsleft: ")[1].split("; "), ["d1 keep a, or drop b", "d2 merge or PR"])

    def test_skip_closes_a_row(self):
        rows = [{"id": "o1", "kind": "optional", "text": "README line", "skip": True},
                {"id": "m1", "kind": "work", "text": "release notes", "after": ["o1"]}]
        res = route.route(rows)
        self.assertEqual(routes(res), {"o1": "SKIP", "m1": "DO"})
        self.assertEqual(res["then"], "mic-drop")

    def test_bad_input(self):
        bad = [
            [{"id": "x", "kind": "nope", "text": "t"}],
            [{"id": "x", "kind": "no-tool", "text": "t"}],                     # tried missing
            [{"id": "x", "kind": "work", "text": "t", "after": ["zz"]}],
            [{"id": "x", "kind": "work", "text": "t"}, {"id": "x", "kind": "work", "text": "u"}],
            [{"id": "x", "kind": "decision", "risk": "push", "text": "t"}],
            [{"id": "x", "kind": "work", "risk": "rm -rf", "text": "t"}],
            {"id": "x"},
        ]
        for rows in bad:
            with self.assertRaises(route.BadInput):
                route.route(rows)

    def test_cli(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = route.main([os.devnull])
        self.assertEqual(code, 2)  # empty file is not JSON
        fd, path = tempfile.mkstemp(suffix=".json")
        try:
            with os.fdopen(fd, "w") as f:
                json.dump(SCENARIO, f)
            buf = io.StringIO()
            with redirect_stdout(buf):
                code = route.main([path])
            self.assertEqual(code, 0)
            self.assertIn("NEXT 2 ps:askme: whatsleft: m3 ", buf.getvalue())
            for bad in (["--cap", "0", path], ["--cap", "-1", path]):
                with redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as cm:
                    route.main(bad)  # argparse exits 2 on a bad option value
                self.assertEqual(cm.exception.code, 2)
        finally:
            os.remove(path)
        with redirect_stdout(io.StringIO()):
            self.assertEqual(route.main([os.path.join(tempfile.gettempdir(), "no-such-rows.json")]), 2)
        fd, path = tempfile.mkstemp(suffix=".json")
        try:
            with os.fdopen(fd, "w") as f:
                json.dump([{"id": 5, "kind": "work", "text": "t"}, {"id": "x", "kind": [1], "text": "t"}], f)
            with redirect_stdout(io.StringIO()):
                self.assertEqual(route.main([path]), 2)  # wrong types are bad input, not a crash
        finally:
            os.remove(path)


class SkillLinkTest(unittest.TestCase):
    """The three skills must keep pointing at each other."""

    def read(self, name):
        with open(os.path.join(SKILLS, name, "SKILL.md"), encoding="utf-8") as f:
            return f.read()

    def test_whatsleft_links(self):
        s = self.read("ps:whatsleft")
        for needle in ("scripts/route.py", "ps:askme", "ps:mic-drop", "AskQuestion",
                       "AskUserQuestion", "whatsleft:"):
            self.assertIn(needle, s)

    def test_askme_links(self):
        s = self.read("ps:askme")
        for needle in ("ps:whatsleft", "whatsleft:", "AskQuestion", "AskUserQuestion",
                       "route.py", "ps:mic-drop"):
            self.assertIn(needle, s)


if __name__ == "__main__":
    unittest.main(verbosity=1)
