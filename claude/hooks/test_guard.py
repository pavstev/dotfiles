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


BRIEF = "Build it. Deadline: 25 min. Hand back at most 4,000 chars; write the rest to a file."


def user_rec(content):
    return {"type": "user", "message": {"role": "user", "content": content}}


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
        self.assertDeny(self.pre("Agent", {"model": "sonnet", "prompt": BRIEF}), 'needs "model" and "effort"')
        self.assertDeny(self.pre("Agent", {"effort": "high", "prompt": BRIEF}), 'needs "model" and "effort"')
        self.assertDeny(self.pre("Agent", {}), "sonnet + high")

    def test_g1_ok(self):
        self.assertAllow(self.pre("Agent", {"model": "sonnet", "effort": "high", "prompt": BRIEF}))

    def test_g1_brief_needs_deadline_and_cap(self):
        base = {"model": "sonnet", "effort": "high"}
        res = self.pre("Agent", dict(base, prompt="build"))
        self.assertDeny(res, "a deadline and a hand-back cap")
        self.assertIn("Deadline: 25 min.", res[2])
        self.assertIn("Hand back at most 4,000 chars; write the rest to a file.", res[2])
        res = self.pre("Agent", dict(base, prompt="Build it. Hand back at most 4,000 chars."))
        self.assertDeny(res, "needs a deadline")
        self.assertNotIn("hand-back cap", res[2])
        res = self.pre("Agent", dict(base, prompt="Build it. Deadline: 25 min."))
        self.assertDeny(res, "needs a hand-back cap")
        self.assertDeny(self.pre("Agent", dict(base)), "a deadline")  # no prompt at all

    def test_g1_brief_accepted_forms(self):
        base = {"model": "sonnet", "effort": "high"}
        for deadline in ("Deadline 30m", "finish in 20 minutes", "within 15 min", "time box 1h", "stop at 5pm"):
            self.assertAllow(self.pre("Agent", dict(base, prompt=deadline + ". Hand back one line.")))
        for cap in ("hand back the status", "at most 3,000 words", "2000 characters", "under 500 lines",
                    "max 4,000 chars", "maximum 800 words", "reply in one line", "reply \u2264 4,000 chars",
                    "reply \u22644000"):
            self.assertAllow(self.pre("Agent", dict(base, prompt="Do it. Deadline: 25 min. " + cap)))
        self.assertAllow(self.pre("Agent", dict(base, prompt="DEADLINE 20 MIN. HAND BACK 3,000 CHARS")))
        for deadline in ("You have 1 hour.", "Time limit: 2 hours.", "a 25-minute budget", "within 30m", "ETA 1h"):
            self.assertAllow(self.pre("Agent", dict(base, prompt=deadline + " Hand back one line.")))
        self.assertAllow(self.pre("Agent", dict(base, prompt="Hand-back: 3000 chars. Time box: yes.")))

    def test_g1_max_needs_keyword(self):
        base = {"model": "opus", "effort": "max", "prompt": ""}

        def b(**kw):
            d = dict(base, **kw)
            d["prompt"] = (d["prompt"] + " " + BRIEF).strip()
            return d

        self.assertDeny(self.pre("Agent", b(description="build it", prompt="do work")), "max")
        self.assertAllow(self.pre("Agent", b(description="Final Review of branch")))
        self.assertAllow(self.pre("Agent", b(description="x", prompt="Write the architecture doc")))
        self.assertAllow(self.pre("Agent", b(description="Plan the refactor")))
        # whole words only: "planet" and "airplane" are not "plan"
        self.assertDeny(self.pre("Agent", b(description="Explain the planet's orbit", prompt="airplane")))
        self.assertDeny(self.pre("Agent", b(description="Implement task 3", prompt="read docs/plans/x.md")))
        self.assertAllow(self.pre("Agent", b(description="Final whole-branch review")))

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
        ok = {"model": "sonnet", "effort": "high", "prompt": BRIEF}
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

    # --- G5 hand-back cap ---
    def sub(self, prompt, agent_id="a1", name=None):
        """Write a subagent transcript under <session>/subagents/ and return the main transcript path."""
        main = self.transcript([human("go")], name="main.jsonl")
        sub_dir = os.path.join(self.dir, "main", "subagents")
        os.makedirs(sub_dir, exist_ok=True)
        with open(os.path.join(sub_dir, "agent-%s.jsonl" % agent_id), "w") as f:
            f.write(json.dumps({"type": "system", "subtype": "init"}) + "\n")
            if prompt is not None:
                f.write(json.dumps(user_rec(prompt)) + "\n")
            f.write(json.dumps(assistant(1000)) + "\n")
        return main

    def handback(self, msg, prompt=None, agent_type=None, agent_id="a1", mode="pretool", **kw):
        data = {"session_id": "s", "agent_id": agent_id, "transcript_path": self.sub(prompt, agent_id)}
        if agent_type:
            data["agent_type"] = agent_type
        data.update(kw)
        if mode == "pretool":
            data.update(hook_event_name="PreToolUse", tool_name="SubagentHandback", tool_input={"message": msg})
        else:
            data.update(hook_event_name="SubagentStop", last_assistant_message=msg)
        return self.run_guard(mode, data)

    def test_g5_default_cap_and_message(self):
        self.assertAllow(self.handback("x" * 4400))  # 4,000 + 10%
        res = self.handback("x" * 4401)
        self.assertDeny(res, "guard: hand-back is 4401 chars; the cap is 4000.")
        self.assertIn("Write the full text to a file", res[2])
        self.assertIn("status on line 1, key results, the file path", res[2])
        self.assertIn("No Write tool: cut to the top findings.", res[2])

    def test_g5_cap_from_prompt(self):
        prompt = "Review it. Hand back at most 1,000 chars; write the rest to a file."
        self.assertAllow(self.handback("x" * 1100, prompt))
        self.assertDeny(self.handback("x" * 1101, prompt), "the cap is 1000")

    def test_g5_smallest_tied_cap_wins(self):
        prompt = "Reply in 2,000 chars. Final message under 1500 characters. Then hand back 800 chars."
        self.assertDeny(self.handback("x" * 900, prompt), "the cap is 800")

    def test_g5_cap_ignored_when_not_tied_to_reply(self):
        prompt = "Write a description of at most 160 characters for the page. " + "z" * 100 + " Do the work."
        self.assertDeny(self.handback("x" * 5000, prompt, agent_id="n1"), "the cap is 4000")
        # small numbers (< 200) never count, even next to a keyword
        self.assertDeny(self.handback("x" * 5000, "Hand back 50 chars.", agent_id="n2"), "the cap is 4000")
        # keyword farther than 80 chars before the number is not a tie
        far = "Reply first." + " " * 90 + "Pad the file to 500 chars."
        self.assertDeny(self.handback("x" * 5000, far, agent_id="n3"), "the cap is 4000")

    def test_g5_cap_phrasings(self):
        for i, (prompt, cap) in enumerate((
            ("Keep your response under 6,000 characters.", 6000),
            ("Limit the output to 5,000 chars.", 5000),
            ("6,000 chars max for the hand-back.", 6000),
            ("Make truncate() return at most 500 characters. Hand back at most 4,000 chars.", 4000),
            ("The API summary field holds 300 chars. Hand back at most 2,000 chars.", 2000),
            ("Hand back at most 4,000 chars. The summary field holds 300 chars.", 4000),
            ("Deadline: answer within 8 minutes. Stay \u2264 4,000 chars.", 4000),
            ("Keep it under 2,500 characters. Pad the file to 900 chars.", 2500),
            ("Hand back \u2264 3,000 chars. In researcher.md set a hard cap of 2,500 chars on the final message.",
             3000),
        )):
            self.assertDeny(self.handback("x" * (int(cap * 1.1) + 1), prompt, agent_id="p%d" % i),
                            "the cap is %d" % cap)

    def test_g5_counter_write_fails_allows(self):
        bad = os.path.join(self.dir, "not-a-dir")
        with open(bad, "w") as f:
            f.write("x")
        data = {"session_id": "s", "hook_event_name": "SubagentStop", "last_assistant_message": "x" * 5000}
        for _ in range(3):
            p = subprocess.run([sys.executable, GUARD, "subagentstop"], input=json.dumps(data),
                               capture_output=True, text=True, env=dict(os.environ, TMPDIR=bad), timeout=30)
            self.assertEqual((p.returncode, p.stderr), (0, ""))

    def test_g5_defaults_by_agent_type(self):
        for kind, cap in (("task-implementer", 3000), ("task-reviewer", 3000), ("code-reviewer", 4000),
                          ("researcher", 3000), ("repo-scanner", 2000), ("ui-verifier", 3000),
                          ("general-purpose", 4000), ("Explore", 4000)):
            self.assertDeny(self.handback("x" * (int(cap * 1.1) + 1), agent_type=kind, agent_id=kind),
                            "the cap is %d" % cap)
            self.assertAllow(self.handback("x" * int(cap * 1.1), agent_type=kind, agent_id=kind + "-ok"))

    def test_g5_two_denies_then_allow(self):
        big = "x" * 5000
        self.assertDeny(self.handback(big))
        self.assertDeny(self.handback(big))
        self.assertAllow(self.handback(big))
        self.assertAllow(self.handback(big))
        # another agent has its own counter
        self.assertDeny(self.handback(big, agent_id="a2"))
        self.assertTrue(os.path.isfile(os.path.join(self.dir, "claude-guard", "handback-a1.json")))

    def test_g5_counter_keyed_by_session_without_agent_id(self):
        data = {"session_id": "sess9", "hook_event_name": "SubagentStop", "last_assistant_message": "x" * 5000}
        self.assertDeny(self.run_guard("subagentstop", data))
        self.assertTrue(os.path.isfile(os.path.join(self.dir, "claude-guard", "handback-sess9.json")))

    def test_g5_subagentstop(self):
        self.assertDeny(self.handback("x" * 5000, mode="subagentstop"), "hand-back is 5000 chars")
        self.assertAllow(self.handback("x" * 100, mode="subagentstop"))
        self.assertAllow(self.handback("x" * 5000, mode="subagentstop", agent_id="b1", stop_hook_active=True))
        self.assertDeny(self.handback("x" * 5000, mode="subagentstop", agent_id="b2", stop_hook_active=False))
        # missing or wrong-typed text allows
        self.assertAllow(self.handback(None, mode="subagentstop", agent_id="b3"))
        self.assertAllow(self.handback(12345, mode="subagentstop", agent_id="b4"))
        self.assertAllow(self.run_guard("subagentstop", {"session_id": "s"}))

    def test_g5_subagentstop_cap_from_prompt(self):
        prompt = "Return a final message of at most 600 characters."
        self.assertDeny(self.handback("x" * 700, prompt, mode="subagentstop"), "the cap is 600")

    def test_g5_transcript_path_variants(self):
        prompt = "Hand back at most 500 chars."
        main = self.sub(prompt, "v1")
        sub = os.path.join(self.dir, "main", "subagents", "agent-v1.jsonl")
        msg = "x" * 1000
        # agent_transcript_path wins
        d = {"session_id": "s", "agent_id": "zz", "transcript_path": "/no/such.jsonl", "agent_transcript_path": sub,
             "last_assistant_message": msg}
        self.assertDeny(self.run_guard("subagentstop", d), "the cap is 500")
        # transcript_path already inside subagents/
        d = {"session_id": "s", "agent_id": "v2", "transcript_path": sub, "last_assistant_message": msg}
        self.assertDeny(self.run_guard("subagentstop", d), "the cap is 500")
        # derived from the main transcript and agent_id
        d = {"session_id": "s", "agent_id": "v1", "transcript_path": main, "last_assistant_message": msg}
        self.assertDeny(self.run_guard("subagentstop", d), "the cap is 500")

    def test_g5_prompt_as_text_blocks_and_first_user_record_only(self):
        main = self.transcript([human("go")], name="m2.jsonl")
        sub_dir = os.path.join(self.dir, "m2", "subagents")
        os.makedirs(sub_dir)
        with open(os.path.join(sub_dir, "agent-t1.jsonl"), "w") as f:
            f.write(json.dumps(user_rec([{"type": "tool_result", "tool_use_id": "x", "content": "y"}])) + "\n")
            f.write(json.dumps(user_rec([{"type": "text", "text": "Hand back at most 400 chars."}])) + "\n")
            f.write(json.dumps(user_rec("Hand back at most 300 chars.")) + "\n")
        d = {"session_id": "s", "agent_id": "t1", "transcript_path": main, "last_assistant_message": "x" * 600}
        self.assertDeny(self.run_guard("subagentstop", d), "the cap is 400")

    def test_g5_prompt_read_from_start_limited_to_200_lines(self):
        main = self.transcript([human("go")], name="m3.jsonl")
        sub_dir = os.path.join(self.dir, "m3", "subagents")
        os.makedirs(sub_dir)
        with open(os.path.join(sub_dir, "agent-l1.jsonl"), "w") as f:
            for _ in range(200):
                f.write(json.dumps(assistant(1000)) + "\n")
            f.write(json.dumps(user_rec("Hand back at most 300 chars.")) + "\n")
        d = {"session_id": "s", "agent_id": "l1", "transcript_path": main, "last_assistant_message": "x" * 600}
        self.assertAllow(self.run_guard("subagentstop", d))  # prompt past line 200 ignored: default cap 4000

    def test_g5_missing_transcript_falls_back_to_default(self):
        d = {"session_id": "s", "agent_id": "q", "transcript_path": "/no/such/main.jsonl",
             "agent_type": "repo-scanner", "last_assistant_message": "x" * 2300}
        self.assertDeny(self.run_guard("subagentstop", d), "the cap is 2000")
        d["last_assistant_message"] = "x" * 100
        self.assertAllow(self.run_guard("subagentstop", d))
        self.assertAllow(self.run_guard("subagentstop", {"agent_id": "q2", "last_assistant_message": "x" * 100}))

    def test_g5_pretool_ignores_other_tools_and_bad_input(self):
        self.assertAllow(self.pre("Read", {"message": "x" * 9000}))
        self.assertAllow(self.pre("SubagentHandback", {}))
        self.assertAllow(self.pre("SubagentHandback", {"message": 42}))
        self.assertAllow(self.run_guard("pretool", {"tool_name": "SubagentHandback", "tool_input": "oops"}))

    # --- scratch mode ---
    def scratch(self, path, key="file_path", **kw):
        data = {"hook_event_name": "PreToolUse", "tool_name": "Write", "tool_input": {key: path}}
        data.update(kw)
        return self.run_guard("scratch", data)

    def test_scratch_allowed_paths(self):
        for p in ("/tmp/x.py", "/private/tmp/a/b.sh", "/var/folders/ab/cd/T/x.js",
                  "/private/var/folders/ab/cd/T/x.js", os.path.join(self.dir, "s.py"),
                  "/Users/someone/.claude/tmp/scratchpad/s.py", "/private/tmp/claude-501/p/abc/scratchpad/s.py",
                  "/Users/someone/proj/.superpowers/sdd/task-9-report.md", "/Users/someone/proj/.agents/notes.md"):
            self.assertAllow(self.scratch(p))
        self.assertAllow(self.scratch(os.path.join(self.dir, "nb.ipynb"), key="notebook_path"))

    def test_scratch_denied_paths(self):
        for p in ("/etc/hosts", os.path.expanduser("~/dotfiles/x.py"), "/tmpfoo/x", "/Users/someone/proj/src/app.ts",
                  "/tmp/../etc/passwd", "relative.py", "/Users/someone/proj/.superpowersx/r.md",
                  "/Users/someone/proj/README.md"):
            self.assertDeny(self.scratch(p, cwd="/Users/someone/proj"),
                            "guard: this agent writes only scratch files; write under the scratchpad, $TMPDIR, "
                            ".agents/ or .superpowers/.")
        self.assertDeny(self.scratch("/etc/x.ipynb", key="notebook_path"), "scratch files")

    def test_scratch_tilde_symlink_and_relative(self):
        self.assertDeny(self.scratch("~/x.py"), "scratch files")
        # a symlink inside the temp dir that points outside is denied
        link = os.path.join(self.dir, "link")
        os.symlink("/etc", link)
        self.assertDeny(self.scratch(os.path.join(link, "hosts")), "scratch files")
        # a relative path resolves against cwd
        self.assertAllow(self.scratch("s.py", cwd=self.dir))
        self.assertDeny(self.scratch("s.py", cwd="/Users/someone/proj"), "scratch files")

    def test_scratch_fail_open(self):
        self.assertAllow(self.run_guard("scratch", raw="not json"))
        self.assertAllow(self.run_guard("scratch", {"tool_name": "Write", "tool_input": {}}))
        self.assertAllow(self.run_guard("scratch", {"tool_name": "Write", "tool_input": "oops"}))
        self.assertAllow(self.run_guard("scratch", {"tool_name": "Write", "tool_input": {"file_path": 5}}))

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
        d = {"tool_name": "Agent", "tool_input": {"model": "m", "effort": "high", "prompt": BRIEF},
             "transcript_path": "/no/such"}
        self.assertAllow(self.run_guard("pretool", d))
        d = {"tool_name": "ScheduleWakeup", "tool_input": {"delaySeconds": 60}, "transcript_path": "/no/such"}
        self.assertAllow(self.run_guard("pretool", d))


if __name__ == "__main__":
    unittest.main(verbosity=1)
