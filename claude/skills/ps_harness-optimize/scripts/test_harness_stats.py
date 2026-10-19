#!/usr/bin/env python3
"""Tests for harness_stats.py (collect + agents). Run: python3 test_harness_stats.py"""
import contextlib
import io
import json
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import harness_stats as hs  # noqa: E402


def usage_rec(ts, ctx, mid, blocks=None):
    return {"type": "assistant", "timestamp": ts, "message": {
        "id": mid, "model": "claude-sonnet-5-5", "content": blocks or [{"type": "text", "text": "ok"}],
        "usage": {"input_tokens": 10, "cache_read_input_tokens": ctx - 10, "cache_creation_input_tokens": 0,
                  "output_tokens": 5}}}


def agent_use(tid, kind=None):
    inp = {"model": "sonnet", "effort": "high", "prompt": "x"}
    if kind:
        inp["subagent_type"] = kind
    return {"type": "tool_use", "id": tid, "name": "Agent", "input": inp}


class AgentsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.projects = os.path.join(self.tmp.name, "projects")
        self.out = os.path.join(self.tmp.name, "rows.json")
        hs.PROJECTS, hs.DESKTOP = self.projects, os.path.join(self.tmp.name, "none")

    def session(self, sid, day, spawns):
        folder = os.path.join(self.projects, "-tmp-proj")
        os.makedirs(os.path.join(folder, sid, "subagents"), exist_ok=True)
        t = "2026-10-%sT10:00:00Z" % day
        recs = [{"type": "user", "timestamp": t, "message": {"role": "user", "content": "go"}},
                usage_rec(t, 20000, sid + "m0", [agent_use("a%d" % i, k) for i, (k, _) in enumerate(spawns)])]
        with open(os.path.join(folder, sid + ".jsonl"), "w") as f:
            f.write("\n".join(json.dumps(r) for r in recs) + "\n")
        for i, (kind, ctx) in enumerate(spawns):
            base = os.path.join(folder, sid, "subagents", "agent-%s%d" % (sid, i))
            with open(base + ".jsonl", "w") as f:
                f.write(json.dumps(usage_rec(t, ctx, "%s-s%d" % (sid, i))) + "\n")
            with open(base + ".meta.json", "w") as f:
                json.dump({"agentType": kind or "general-purpose", "model": "sonnet"}, f)

    def run_cmd(self, *argv):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            hs.main(list(argv))
        return buf.getvalue()

    def test_spawns_and_agents_table(self):
        self.session("s1", "10", [(None, 60000), ("general-purpose", 64000), ("researcher", 15000)])
        self.session("s2", "16", [("worker", 14000), ("worker", 16000)])
        self.run_cmd("collect", "--since", "2026-10-01", "--out", self.out)
        with open(self.out) as f:
            rows = {r["sid"]: r for r in json.load(f)}
        self.assertEqual(sorted(rows["s1"]["sub"]["spawns"]),
                         [["general-purpose", 60000], ["general-purpose", 64000], ["researcher", 15000]])
        self.assertEqual([c["type"] for c in rows["s1"]["agent_calls"]], ["general-purpose", "general-purpose", "researcher"])
        out = self.run_cmd("agents", "--in", self.out, "--cut", "2026-10-15T00:00:00+00:00")
        self.assertIn("| general-purpose | 2 (67%) | 62 | 0 (0%) | - |", out)
        self.assertIn("| worker | 0 (0%) | - | 2 (100%) | 15 |", out)
        self.assertIn("Sessions: P0 1, P1 1", out)
        self.assertIn("Early (under 20 sessions): P0, P1.", out)
        rep = self.run_cmd("report", "--in", self.out, "--cut", "2026-10-15T00:00:00+00:00")
        self.assertIn("| general-purpose spawns % | 67 | 0 |", rep)


class NullTokensTest(unittest.TestCase):
    def test_scan_survives_null_token_fields(self):
        rec = usage_rec("2026-10-10T10:00:00Z", 1000, "m1")
        rec["message"]["usage"].update({"input_tokens": None, "cache_creation_input_tokens": None,
                                        "output_tokens": None})
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "s.jsonl")
            with open(path, "w") as f:
                f.write(json.dumps(rec) + "\n")
            res = hs.scan(path)
        self.assertEqual(res["r"]["calls"], 1)
        self.assertEqual(res["r"]["out"], 0)
        self.assertEqual(res["r"]["fresh_in"], 0)
        self.assertEqual(res["r"]["cache_read"], 990)
        self.assertEqual(res["peak"], 990)


if __name__ == "__main__":
    unittest.main(verbosity=1)
