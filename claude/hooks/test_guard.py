#!/usr/bin/env python3
"""Tests for guard.py. Run: python3 claude/hooks/test_guard.py"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
GUARD = os.path.join(HERE, "guard.py")
sys.path.insert(0, HERE)
import guard  # noqa: E402


def assistant(total, blocks=None):
    return {
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": blocks or [{"type": "text", "text": "ok"}],
            "usage": {
                "input_tokens": 10,
                "cache_creation_input_tokens": total - 10 - 500,
                "cache_read_input_tokens": 500,
                "output_tokens": 5,
            },
        },
    }


def human(text="hi"):
    return {"type": "user", "message": {"role": "user", "content": text}}


def tool_result(tid="t1"):
    return {
        "type": "user",
        "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tid, "content": "x"}]},
    }


def ask_block(tid, q="which?"):
    return {"type": "tool_use", "id": tid, "name": "AskUserQuestion", "input": {"questions": [q]}}


class GuardTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = self.tmp.name

    def transcript(self, records, name="t.jsonl"):
        path = os.path.join(self.dir, name)
        with open(path, "w") as f:
            for r in records:
                f.write(json.dumps(r) + "\n")
        return path

    def run_guard(self, mode, data=None, raw=None):
        env = dict(os.environ, TMPDIR=self.dir)
        stdin = raw if raw is not None else json.dumps(data)
        p = subprocess.run(
            [sys.executable, GUARD, mode], input=stdin, capture_output=True, text=True, env=env, timeout=30
        )
        return p.returncode, p.stdout, p.stderr

    def pre(self, tool, tool_input, ctx=0, extra=None, **kw):
        path = self.transcript([human(), assistant(ctx)]) if ctx else None
        data = {"hook_event_name": "PreToolUse", "session_id": "s", "tool_name": tool, "tool_input": tool_input}
        if path:
            data["transcript_path"] = path
        data.update(kw)
        if extra:
            data.update(extra)
        return self.run_guard("pretool", data)

    def assertDeny(self, res, text=""):
        self.assertEqual(res[0], 2, res)
        self.assertIn(text, res[2])
        self.assertEqual(res[1], "")

    def assertAllow(self, res):
        self.assertEqual(res, (0, "", ""), res)

    # --- G1 ---
    def test_g1_missing_effort_or_model(self):
        self.assertDeny(self.pre("Agent", {"model": "sonnet", "prompt": "x"}), 'needs "model" and "effort"')
        self.assertDeny(self.pre("Agent", {"effort": "high", "prompt": "x"}), 'needs "model" and "effort"')
        self.assertDeny(self.pre("Agent", {}), "sonnet + high")

    def test_g1_ok(self):
        self.assertAllow(self.pre("Agent", {"model": "sonnet", "effort": "high", "prompt": "build"}))

    def test_g1_max_needs_keyword(self):
        base = {"model": "opus", "effort": "max"}
        self.assertDeny(self.pre("Agent", dict(base, description="build it", prompt="do work")), "max")
        self.assertAllow(self.pre("Agent", dict(base, description="Final Review of branch")))
        self.assertAllow(self.pre("Agent", dict(base, description="x", prompt="Write the architecture doc")))
        self.assertAllow(self.pre("Agent", dict(base, description="Plan the refactor")))
        # whole words only: "planet" and "airplane" are not "plan"
        self.assertDeny(self.pre("Agent", dict(base, description="Explain the planet's orbit", prompt="airplane")))
        self.assertDeny(self.pre("Agent", dict(base, description="Implement task 3", prompt="read docs/plans/x.md")))
        self.assertAllow(self.pre("Agent", dict(base, description="Final whole-branch review")))

    # --- G2 ---
    def test_g2_cron(self):
        self.assertDeny(self.pre("CronCreate", {"cron": "*/5 * * * *", "prompt": "p"}, 150_000), "no polling")
        self.assertAllow(self.pre("CronCreate", {"cron": "7 * * * *", "prompt": "p"}, 150_000))
        self.assertAllow(self.pre("CronCreate", {"cron": "*/5 * * * *", "prompt": "p"}, 50_000))
        self.assertAllow(self.pre("CronCreate", {"cron": "*/5 * * * *", "prompt": "p"}))  # no transcript

    def test_g2_cron_one_shot(self):
        self.assertAllow(self.pre("CronCreate", {"cron": "* * * * *", "prompt": "p", "recurring": False}, 150_000))
        self.assertDeny(self.pre("CronCreate", {"cron": "* * * * *", "prompt": "p", "recurring": True}, 150_000))

    def test_g2_cron_lists_and_hours(self):
        deny = self.assertDeny
        deny(self.pre("CronCreate", {"cron": "0,10,20 * * * *"}, 150_000))
        deny(self.pre("CronCreate", {"cron": "0,50 * * * *"}, 150_000))  # wrap gap 10
        self.assertAllow(self.pre("CronCreate", {"cron": "0,30 * * * *"}, 150_000))
        self.assertAllow(self.pre("CronCreate", {"cron": "0 */2 * * *"}, 150_000))
        self.assertAllow(self.pre("CronCreate", {"cron": "garbage"}, 150_000))

    def test_cron_interval_function(self):
        f = guard.cron_interval_minutes
        self.assertEqual(f("* * * * *"), 1)
        self.assertEqual(f("*/15 * * * *"), 15)
        self.assertEqual(f("5 * * * *"), 60)
        self.assertEqual(f("0-30/10 * * * *"), 10)
        self.assertEqual(f("0 */3 * * *"), 180)
        self.assertEqual(f("0 9 * * *"), None)
        self.assertEqual(f("nope"), None)

    def test_g2_wakeup(self):
        self.assertDeny(self.pre("ScheduleWakeup", {"delaySeconds": 600}, 150_000), "no polling")
        self.assertAllow(self.pre("ScheduleWakeup", {"delaySeconds": 1800}, 150_000))
        self.assertAllow(self.pre("ScheduleWakeup", {"delaySeconds": 600}, 50_000))

    def test_g2_loop_skill(self):
        self.assertDeny(self.pre("Skill", {"skill": "loop", "args": "5m check deploy"}, 150_000), "no polling")
        self.assertDeny(self.pre("Skill", {"skill": "loop", "args": "30s /foo"}, 150_000))
        self.assertDeny(self.pre("Skill", {"skill": "loop", "args": "10min x"}, 150_000))
        self.assertAllow(self.pre("Skill", {"skill": "loop", "args": "45m check deploy"}, 150_000))
        self.assertAllow(self.pre("Skill", {"skill": "loop", "args": "1h x"}, 150_000))
        self.assertAllow(self.pre("Skill", {"skill": "loop", "args": "2h30m x"}, 150_000))
        self.assertAllow(self.pre("Skill", {"skill": "loop", "args": "check the deploy"}, 150_000))
        self.assertAllow(self.pre("Skill", {"skill": "loop"}, 150_000))
        self.assertAllow(self.pre("Skill", {"skill": "other", "args": "5m x"}, 150_000))
        self.assertAllow(self.pre("Skill", {"skill": "loop", "args": "5m x"}, 50_000))
        self.assertDeny(self.pre("Skill", {"skill": "loop", "args": "check deploy every 5m"}, 150_000))
        self.assertAllow(self.pre("Skill", {"skill": "loop", "args": "check deploy every 45m"}, 150_000))

    # --- G3 ---
    def prompt_run(self, ctx, sid="sess"):
        path = self.transcript([human(), assistant(ctx)], name=f"{sid}.jsonl")
        return self.run_guard(
            "prompt",
            {"hook_event_name": "UserPromptSubmit", "session_id": sid, "transcript_path": path, "prompt": "go"},
        )

    def test_g3_prompt_cadence(self):
        rc, out, err = self.prompt_run(350_000)
        self.assertEqual((rc, err), (0, ""))
        payload = json.loads(out)["hookSpecificOutput"]
        self.assertEqual(payload["hookEventName"], "UserPromptSubmit")
        self.assertEqual(
            payload["additionalContext"],
            "Context is 350K. Run /ps:handoff and continue in a fresh session.",
        )
        for _ in range(4):
            self.assertAllow(self.prompt_run(350_000))
        rc, out, _ = self.prompt_run(350_000)  # 6th
        self.assertEqual(rc, 0)
        self.assertIn("additionalContext", out)
        self.assertAllow(self.prompt_run(350_000))

    def test_g3_prompt_below_threshold_and_sessions_independent(self):
        self.assertAllow(self.prompt_run(299_000))
        rc, out, _ = self.prompt_run(300_000, sid="a")
        self.assertIn("300K", out)
        rc, out, _ = self.prompt_run(300_000, sid="b")
        self.assertIn("300K", out)
        self.assertTrue(os.path.isfile(os.path.join(self.dir, "claude-guard", "a.json")))

    def test_g3_agent_ceiling(self):
        ok = {"model": "sonnet", "effort": "high"}
        res = self.pre("Agent", ok, 600_000)
        self.assertDeny(res, "context is 600K (> 500K)")
        self.assertIn("/ps:handoff", res[2])
        self.assertAllow(self.pre("Agent", ok, 400_000))
        # ceiling runs before G1
        self.assertDeny(self.pre("Agent", {}, 600_000), "context is 600K")

    # --- G4 ---
    def ask(self, records, tool_use_id=None, q="which?"):
        path = self.transcript(records)
        extra = {"tool_use_id": tool_use_id} if tool_use_id else None
        data = {"transcript_path": path}
        return self.run_guard(
            "pretool",
            dict(
                data,
                tool_name="AskUserQuestion",
                tool_input={"questions": [q]},
                **(extra or {}),
            ),
        )

    def test_g4_first_ask_allowed_even_if_written(self):
        recs = [human("start"), assistant(1000, [ask_block("a1")])]
        self.assertAllow(self.ask(recs, tool_use_id="a1"))
        self.assertAllow(self.ask(recs))  # no id: matched by input
        self.assertAllow(self.ask([human("start"), assistant(1000)], tool_use_id="a1"))  # not yet written

    def test_g4_second_ask_denied(self):
        recs = [
            human("start"),
            assistant(1000, [ask_block("a1")]),
            tool_result("a1"),
            assistant(1000, [ask_block("a2")]),
        ]
        self.assertDeny(self.ask(recs, tool_use_id="a2"), "one AskUserQuestion per turn")
        self.assertDeny(self.ask(recs))  # no id: a1 still counted
        # current call not yet written, earlier one counts
        self.assertDeny(self.ask(recs[:3], tool_use_id="a2"))

    def test_g4_parallel_asks_first_allowed(self):
        recs = [human("start"), assistant(1000, [ask_block("a1"), ask_block("a2", "other?")])]
        self.assertAllow(self.ask(recs, tool_use_id="a1"))
        self.assertDeny(self.ask(recs, tool_use_id="a2", q="other?"), "one AskUserQuestion per turn")

    def test_g4_denied_ask_does_not_count(self):
        denied = {
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "a1", "is_error": True, "content": "guard: denied"}]},
        }
        recs = [human("start"), assistant(1000, [ask_block("a1")]), denied, assistant(1000, [ask_block("a2")])]
        self.assertAllow(self.ask(recs, tool_use_id="a2"))

    def test_g4_askme_interview_exempt(self):
        two = [assistant(1000, [ask_block("a1")]), tool_result("a1"), assistant(1000, [ask_block("a2")])]
        self.assertAllow(self.ask([human("/ps:askme plan the move")] + two, tool_use_id="a2"))
        skill = assistant(1000, [{"type": "tool_use", "id": "s1", "name": "Skill", "input": {"skill": "ps:askme"}}])
        self.assertAllow(self.ask([human("grill me"), skill] + two, tool_use_id="a2"))

    def test_g4_new_human_prompt_resets(self):
        recs = [
            human("start"),
            assistant(1000, [ask_block("a1")]),
            tool_result("a1"),
            human("answer"),
            assistant(1000, [ask_block("a2")]),
        ]
        self.assertAllow(self.ask(recs, tool_use_id="a2"))

    def test_g4_list_content_human_and_meta_skipped(self):
        list_prompt = {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "hey"}]}}
        recs = [
            human("start"),
            assistant(1000, [ask_block("a1")]),
            list_prompt,
            assistant(1000, [ask_block("a2")]),
        ]
        self.assertAllow(self.ask(recs, tool_use_id="a2"))
        meta = dict(human("<system-reminder>"), isMeta=True)
        compact = dict(human("summary"), isCompactSummary=True)
        recs = [human("start"), assistant(1000, [ask_block("a1")]), meta, compact, assistant(1000, [ask_block("a2")])]
        self.assertDeny(self.ask(recs, tool_use_id="a2"))

    # --- transcript reading ---
    def test_last_assistant_usage_used_and_large_file(self):
        recs = [human()]
        for i in range(3000):
            recs.append(assistant(120_000))
            recs.append(tool_result())
        recs.append(assistant(250_000))
        recs.append(tool_result())  # last record has no usage
        path = self.transcript(recs)
        self.assertEqual(guard.context_size(path), 250_000)
        self.assertEqual(guard.context_size(os.path.join(self.dir, "nope.jsonl")), 0)

    # --- fail-open ---
    def test_fail_open(self):
        self.assertAllow(self.run_guard("pretool", raw="not json {{{"))
        self.assertAllow(self.run_guard("pretool", raw=""))
        self.assertAllow(self.run_guard("pretool", raw="[1,2]"))
        self.assertAllow(self.run_guard("prompt", raw="garbage"))
        self.assertAllow(self.run_guard("bogus", {"tool_name": "Agent", "tool_input": {}}))
        self.assertAllow(self.run_guard("", {}))
        bad = {"tool_name": "AskUserQuestion", "transcript_path": os.path.join(self.dir, "missing.jsonl")}
        self.assertAllow(self.run_guard("pretool", bad))
        self.assertAllow(self.run_guard("pretool", {"tool_name": "Agent", "tool_input": "oops-string"}))
        self.assertAllow(self.run_guard("pretool", {"tool_name": "Read", "tool_input": {}}))

    def test_missing_transcript_means_zero_context(self):
        d = {"tool_name": "Agent", "tool_input": {"model": "m", "effort": "high"}, "transcript_path": "/no/such"}
        self.assertAllow(self.run_guard("pretool", d))
        d = {"tool_name": "ScheduleWakeup", "tool_input": {"delaySeconds": 60}, "transcript_path": "/no/such"}
        self.assertAllow(self.run_guard("pretool", d))


if __name__ == "__main__":
    unittest.main(verbosity=1)
