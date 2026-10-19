#!/usr/bin/env python3
"""Tests for guard.py. Run: python3 claude/hooks/test_guard.py"""
import io
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import types
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
GUARD = os.path.join(HERE, "guard.py")
sys.path.insert(0, HERE)
import guard  # noqa: E402


def fake_pace(state="ok", used=40.0, budget=50.0, calls=None):
    """A stand-in for the pace module (claude/hooks/pace.py, contract C2)."""

    def status(now=None):
        return {"state": state, "used": used, "budget": budget, "source": "estimate", "resets_at": None}

    def refresh(budget_s=5):
        if calls is not None:
            calls.append(budget_s)

    return types.SimpleNamespace(status=status, refresh=refresh)


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
        env = mock.patch.dict(os.environ)  # restored after each test
        env.start()
        self.addCleanup(env.stop)
        os.environ.pop("CLAUDE_CODE_ENTRYPOINT", None)  # a desktop session running the tests must not mute G13
        # the G6 hand-off must exist on disk: Stop and archive skip a deleted one
        self.HO = os.path.join(self.dir, "repo", ".agents", "handoffs", "2026-10-14-x.md")
        os.makedirs(os.path.dirname(self.HO))
        with open(self.HO, "w") as f:
            f.write("# Task")

    def transcript(self, records, name="t.jsonl"):
        path = os.path.join(self.dir, name)
        with open(path, "w") as f:
            for r in records:
                f.write(json.dumps(r) + "\n")
        return path

    def run_guard(self, mode, data=None, raw=None):
        # HOME and the cache point at the temp dir: pace finds no data there and answers "unknown"
        env = dict(os.environ, TMPDIR=self.dir)
        if mode != "scratch":  # scratch resolves "~" and must keep the real HOME
            env.update(HOME=self.dir, XDG_CACHE_HOME=os.path.join(self.dir, "cache"))
        stdin = raw if raw is not None else json.dumps(data)
        p = subprocess.run(
            [sys.executable, GUARD, mode], input=stdin, capture_output=True, text=True, env=env, timeout=30
        )
        return p.returncode, p.stdout, p.stderr

    def inproc(self, mode, data, pace=None):
        """Run guard.main in this process with a mocked pace module; returns (code, stdout, stderr)."""
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.object(guard, "pace", pace), mock.patch.dict(os.environ, {"TMPDIR": self.dir}), \
                mock.patch.object(sys, "argv", ["guard.py", mode]), \
                mock.patch.object(sys, "stdin", io.StringIO(json.dumps(data))), \
                mock.patch.object(sys, "stdout", out), mock.patch.object(sys, "stderr", err):
            code = guard.main()
        return code, out.getvalue(), err.getvalue()

    _sid = 0

    def pre(self, tool, tool_input, ctx=0, extra=None, **kw):
        path = self.transcript([human(), assistant(ctx)]) if ctx else None
        if tool == "Agent" and isinstance(tool_input, dict) and "subagent_type" not in tool_input:
            tool_input = dict(tool_input, subagent_type="worker")  # not a refused type
        GuardTest._sid += 1  # a fresh session per call: the concurrency cap counts per session
        data = {"hook_event_name": "PreToolUse", "session_id": "s%d" % GuardTest._sid, "tool_name": tool,
                "tool_input": tool_input}
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

    def assertWarn(self, res, text=""):
        self.assertEqual(res[0], 0, res)
        self.assertEqual(res[2], "", res)
        out = json.loads(res[1])["hookSpecificOutput"]
        self.assertEqual(out["hookEventName"], "PreToolUse")
        self.assertIn(text, out["additionalContext"])
        return out["additionalContext"]

    # --- heavy agents are refused ---
    def test_heavy_agents_denied(self):
        base = {"model": "sonnet", "effort": "high", "prompt": BRIEF}
        for kind in ("general-purpose", None, "claude"):
            res = self.pre("Agent", dict(base, subagent_type=kind))
            self.assertDeny(res, "`worker`")
            for name in ("`researcher`", "`Explore`", "`task-implementer`", "`Plan`"):
                self.assertIn(name, res[2])
        # a missing subagent_type key is the same as general-purpose
        data = {"session_id": "h1", "tool_name": "Agent", "tool_input": base}
        self.assertDeny(self.run_guard("pretool", data), "`worker`")
        for kind in ("worker", "task-implementer", "code-reviewer", "researcher"):
            self.assertAllow(self.pre("Agent", dict(base, subagent_type=kind)))

    # --- router ---
    def route(self, kind, model="sonnet", effort="high", desc="Do it", prompt=BRIEF, pace=None, **more):
        ti = {"subagent_type": kind, "description": desc, "prompt": prompt}
        if model is not None:
            ti["model"] = model
        if effort is not None:
            ti["effort"] = effort
        ti.update(more)
        data = {"hook_event_name": "PreToolUse", "session_id": "r%d" % time.time_ns(), "tool_name": "Agent",
                "tool_input": ti}
        if pace is not None:
            res = self.inproc("pretool", data, pace)
        else:
            res = self.run_guard("pretool", data)
        self.assertEqual(res[0], 0, res)
        self.assertEqual(res[2], "", res)
        return ti, json.loads(res[1])["hookSpecificOutput"] if res[1] else None

    def assertRouted(self, out, ti, model, effort):
        self.assertEqual(out["hookEventName"], "PreToolUse")
        self.assertEqual(out["permissionDecision"], "allow")
        self.assertTrue(out["additionalContext"])
        new = out["updatedInput"]
        self.assertEqual(new["model"], model)
        if effort is None:
            self.assertNotIn("effort", new)
        else:
            self.assertEqual(new["effort"], effort)
        for k, v in ti.items():  # every other field is copied
            if k not in ("model", "effort"):
                self.assertEqual(new[k], v)

    def test_router_table(self):
        ti, out = self.route("Plan", model="sonnet", effort="high")
        self.assertRouted(out, ti, "opus", "high")
        ti, out = self.route("code-reviewer", desc="Final whole-branch review")
        self.assertRouted(out, ti, "opus", "high")
        ti, out = self.route("code-reviewer", desc="review", prompt="Final review of main..HEAD. " + BRIEF)
        self.assertRouted(out, ti, "opus", "high")
        for kind in ("Explore", "claude-code-guide", "statusline-setup"):
            ti, out = self.route(kind)
            self.assertRouted(out, ti, "haiku", None)
        ti, out = self.route("worker", model="opus", effort="max")
        self.assertRouted(out, ti, "sonnet", "high")
        ti, out = self.route("researcher", model=None, effort=None)
        self.assertRouted(out, ti, "sonnet", "high")
        ti, out = self.route("task-implementer", effort="xhigh")
        self.assertRouted(out, ti, "sonnet", "high")

    def test_router_review_cases(self):
        # a per-task or plain review stays on sonnet; panel angles and re-reviews too
        for kind, desc, prompt in (("code-reviewer", "Review commit abc", BRIEF),
                                   ("task-reviewer", "Review task 3", "Final review of main..HEAD. " + BRIEF),
                                   ("code-reviewer", "panel: simplify", "Final review of main..HEAD. " + BRIEF),
                                   ("code-reviewer", "Re-review blocking findings", "Final review. " + BRIEF)):
            ti, out = self.route(kind, model="opus", desc=desc, prompt=prompt)
            self.assertRouted(out, ti, "sonnet", "high")
        ti, out = self.route("worker", model="opus", desc="final review of the branch")
        self.assertRouted(out, ti, "sonnet", "high")
        # "final review" deep in an implementer brief is not a review call
        ti, out = self.route("task-implementer", model="opus", prompt=BRIEF + " A final review follows.")
        self.assertRouted(out, ti, "sonnet", "high")

    def test_router_never_raises_cost(self):
        # a caller's cheaper model and low/medium effort stay
        ti, out = self.route("worker", model="haiku", effort="low")
        self.assertRouted(out, ti, "haiku", None)  # haiku takes no effort
        ti, out = self.route("worker", model="sonnet", effort="low")
        self.assertIsNone(out)
        ti, out = self.route("worker", model="claude-sonnet-4-5", effort="medium")
        self.assertIsNone(out)
        ti, out = self.route("task-implementer", model=None, effort="medium")
        self.assertRouted(out, ti, "sonnet", "medium")
        ti, out = self.route("worker", model="haiku", effort="high")
        self.assertRouted(out, ti, "haiku", None)
        ti, out = self.route("worker", model="opus", effort="max")
        self.assertRouted(out, ti, "sonnet", "high")
        ti, out = self.route("worker", model="fable", effort="xhigh")
        self.assertRouted(out, ti, "sonnet", "high")
        ti, out = self.route("worker", model="sonnet", effort="max")
        self.assertRouted(out, ti, "sonnet", "high")
        ti, out = self.route("worker", model="some-unknown-model", effort="low")
        self.assertRouted(out, ti, "sonnet", "low")
        # lookups never go above haiku
        ti, out = self.route("Explore", model="opus", effort="medium")
        self.assertRouted(out, ti, "haiku", None)

    def test_router_opus_rows_follow_the_table(self):
        ti, out = self.route("Plan", model="sonnet", effort="low")
        self.assertRouted(out, ti, "opus", "high")
        ti, out = self.route("Plan", model="haiku", effort=None)
        self.assertRouted(out, ti, "opus", "high")
        ti, out = self.route("code-reviewer", model="sonnet", effort="medium", desc="Final whole-branch review")
        self.assertRouted(out, ti, "opus", "high")
        ahead = fake_pace("ahead", used=60.0, budget=50.0)
        ti, out = self.route("Plan", model="sonnet", effort="low", pace=ahead)
        self.assertRouted(out, ti, "sonnet", "high")
        ti, out = self.route("Plan", model="opus", effort="max", pace=ahead)
        self.assertRouted(out, ti, "sonnet", "high")

    def test_router_silent_when_nothing_changes(self):
        for kind, model, effort in (("worker", "sonnet", "high"), ("Plan", "opus", "high"), ("Explore", "haiku", None)):
            _, out = self.route(kind, model=model, effort=effort)
            self.assertIsNone(out)

    def test_router_skips_fork_and_subagents(self):
        _, out = self.route("fork", model=None, effort=None)
        self.assertIsNone(out)
        res = self.pre("Agent", {"subagent_type": "worker", "model": "opus", "effort": "max", "prompt": BRIEF},
                       extra={"agent_id": "a1"})
        self.assertAllow(res)

    def test_router_pace_ahead_downgrades_opus(self):
        pace = fake_pace("ahead", used=60.0, budget=50.0)
        ti, out = self.route("Plan", model="opus", pace=pace)
        self.assertRouted(out, ti, "sonnet", "high")
        self.assertIn("ahead", out["additionalContext"])
        ti, out = self.route("code-reviewer", model="opus", desc="Final review", pace=pace)
        self.assertRouted(out, ti, "sonnet", "high")
        ti, out = self.route("Explore", pace=pace)
        self.assertRouted(out, ti, "haiku", None)
        # at ok the same Plan call is opus
        ti, out = self.route("Plan", pace=fake_pace("ok"))
        self.assertRouted(out, ti, "opus", "high")

    def test_router_merges_with_warnings(self):
        # W4 note and the router note come out as one JSON line
        path = self.spawn_session(15)
        data = {"hook_event_name": "PreToolUse", "session_id": "m1", "tool_name": "Agent", "tool_use_id": "new",
                "transcript_path": path,
                "tool_input": {"subagent_type": "worker", "model": "opus", "effort": "max", "prompt": BRIEF}}
        rc, out, err = self.run_guard("pretool", data)
        self.assertEqual((rc, err, out.count("\n")), (0, "", 1))
        hso = json.loads(out)["hookSpecificOutput"]
        self.assertEqual(hso["updatedInput"]["model"], "sonnet")
        self.assertIn("spawned 15 agents", hso["additionalContext"])

    # --- W3 browser calls in a main window ---
    def browser_turn(self, n, older=0, name="mcp__Claude_Browser__get_page_text"):
        recs = [human("old")]
        recs += [assistant(1000, [{"type": "tool_use", "id": "o%d" % i, "name": name, "input": {}}]) for i in range(older)]
        recs.append(human("check the page"))
        recs += [assistant(1000, [{"type": "tool_use", "id": "b%d" % i, "name": name, "input": {}}]) for i in range(n)]
        return self.transcript(recs)

    def browser_call(self, path, **kw):
        data = {"hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "mcp__Claude_Browser__navigate",
                "tool_input": {"url": "https://example.com"}, "tool_use_id": "now", "transcript_path": path}
        data.update(kw)
        return self.run_guard("pretool", data)

    def test_w3_browser(self):
        self.assertAllow(self.browser_call(self.browser_turn(4)))
        self.assertWarn(self.browser_call(self.browser_turn(5)), "`ui-verifier`")
        self.assertAllow(self.browser_call(self.browser_turn(6)))
        self.assertWarn(self.browser_call(self.browser_turn(25)), "26 browser calls")
        self.assertAllow(self.browser_call(self.browser_turn(2, older=10)))  # last turn does not count
        self.assertWarn(self.browser_call(self.browser_turn(5, name="mcp__claude-in-chrome__computer")))
        self.assertAllow(self.browser_call(self.browser_turn(5), agent_id="a1"))  # inside ui-verifier
        self.assertAllow(self.browser_call(None))

    # --- W4 budget ---
    def spawn_session(self, n, tokens_each=0):
        recs = [human()]
        for i in range(n):
            recs.append(assistant(1000, [{"type": "tool_use", "id": "a%d" % i, "name": "Agent", "input": {}}]))
            res = tool_result("a%d" % i)
            if tokens_each:
                res["toolUseResult"] = {"totalTokens": tokens_each, "agentType": "worker"}
            recs.append(res)
        return self.transcript(recs)

    def spawn(self, path, tid="new"):
        GuardTest._sid += 1
        data = {"hook_event_name": "PreToolUse", "session_id": "w%d" % GuardTest._sid, "tool_name": "Agent",
                "tool_use_id": tid,
                "transcript_path": path,
                "tool_input": {"subagent_type": "worker", "model": "sonnet", "effort": "high", "prompt": BRIEF}}
        return self.run_guard("pretool", data)

    def test_w4_budget(self):
        self.assertAllow(self.spawn(self.spawn_session(14)))
        self.assertWarn(self.spawn(self.spawn_session(15)), "spawned 15 agents")
        self.assertAllow(self.spawn(self.spawn_session(16)))
        self.assertWarn(self.spawn(self.spawn_session(20)), "get_usage")
        self.assertWarn(self.spawn(self.spawn_session(5, tokens_each=400_000)), "2000K subagent tokens")
        self.assertAllow(self.spawn(self.spawn_session(6, tokens_each=400_000)))
        self.assertAllow(self.spawn(self.spawn_session(5, tokens_each=100_000)))
        # background agents report in task-notification attachments, once per task id
        path = self.spawn_session(5)
        with open(path, "a") as f:
            for tid in ("t1", "t1", "t2"):
                f.write(json.dumps({"type": "attachment", "attachment": {
                    "type": "queued_command", "commandMode": "task-notification",
                    "prompt": "<task-notification> <task-id>%s</task-id>" % tid,
                    "usage": {"totalTokens": 800_000}}}) + "\n")
        self.assertWarn(self.spawn(path), "1600K subagent tokens")
        # the current call already written to the transcript is not counted twice
        self.assertWarn(self.spawn(self.spawn_session(16), tid="a15"), "spawned 15 agents")

    # --- G1 ---
    def test_g1_missing_effort_or_model_is_filled_by_router(self):
        for ti in ({"model": "sonnet", "prompt": BRIEF}, {"effort": "high", "prompt": BRIEF}, {"prompt": BRIEF}):
            res = self.pre("Agent", ti)
            self.assertEqual(res[0], 0, res)
            new = json.loads(res[1])["hookSpecificOutput"]["updatedInput"]
            self.assertEqual((new["model"], new["effort"]), ("sonnet", "high"))
        # the brief check still runs
        self.assertDeny(self.pre("Agent", {}), "a deadline and a hand-back cap")

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

    def test_g1_max_effort_is_clamped_not_denied(self):
        for effort in ("max", "xhigh"):
            res = self.pre("Agent", {"model": "opus", "effort": effort, "prompt": BRIEF, "description": "build it"})
            self.assertEqual(res[0], 0, res)
            new = json.loads(res[1])["hookSpecificOutput"]["updatedInput"]
            self.assertEqual((new["model"], new["effort"]), ("sonnet", "high"))

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
            "Context is 350K. Keep PROGRESS.md (or the run's ledger) current; auto-compact carries the "
            "run. Hand off only when the user asks.",
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
        self.assertEqual(guard.SPAWN_CONTEXT, 350_000)
        res = self.pre("Agent", ok, 600_000)
        self.assertDeny(res, "context is 600K (> 350K)")
        self.assertNotIn("/ps:handoff", res[2])
        self.assertDeny(self.pre("Agent", ok, 351_000), "context is 351K")
        self.assertAllow(self.pre("Agent", ok, 340_000))
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

    # --- G6 hand-off ---
    HO = "/repo/.agents/handoffs/2026-10-14-x.md"

    def write_ho(self, tid="w1", path=None):
        return assistant(1000, [{"type": "tool_use", "id": tid, "name": "Write",
                                 "input": {"file_path": path or self.HO, "content": "# Task"}}])

    def say(self, text):
        return assistant(1000, [{"type": "text", "text": text}])

    def good_reply(self):
        return self.say(f"Done. [2026-10-14-x.md](.agents/handoffs/2026-10-14-x.md)\n```\nRead {self.HO} and do it.\n```")

    def stop_run(self, recs, **extra):
        data = {"hook_event_name": "Stop", "session_id": "s", "transcript_path": self.transcript(recs)}
        data.update(extra)
        return self.run_guard("stop", data)

    def archive(self, recs, sid="self"):
        return self.run_guard("pretool", {"hook_event_name": "PreToolUse", "session_id": "s",
                                          "tool_name": "mcp__ccd_session_mgmt__archive_session",
                                          "tool_input": {"session_id": sid},
                                          "transcript_path": self.transcript(recs)})

    def test_g6_stop_needs_link_and_block(self):
        base = [human("hand off"), self.write_ho(), tool_result("w1")]
        self.assertDeny(self.stop_run(base + [self.say("The hand-off is written.")]), "Read " + self.HO)
        self.assertDeny(self.stop_run(base + [self.say(f"Read {self.HO} and do it.")]), "file link")
        self.assertAllow(self.stop_run(base + [self.good_reply()]))
        # last_assistant_message counts when the transcript is not flushed yet
        self.assertAllow(self.stop_run(base, last_assistant_message=self.good_reply()["message"]["content"][0]["text"]))

    def test_g6_stop_one_deny_and_scope(self):
        base = [human("hand off"), self.write_ho(), tool_result("w1"), self.say("written")]
        self.assertAllow(self.stop_run(base, stop_hook_active=True))
        self.assertAllow(self.stop_run([human("x"), self.say("no hand-off here")]))
        # written in an earlier turn: the user already got it
        self.assertAllow(self.stop_run(base + [human("thanks"), self.say("ok")]))
        # a Write elsewhere is not a hand-off
        self.assertAllow(self.stop_run([human("x"), self.write_ho(path="/repo/notes.md"), self.say("ok")]))

    def test_g6_archive_same_turn_denied(self):
        base = [human("hand off and archive"), self.write_ho(), tool_result("w1"), self.good_reply()]
        self.assertDeny(self.archive(base), "Do not archive yet")
        self.assertAllow(self.archive(base + [human("ok, archive")]))
        self.assertAllow(self.archive(base, sid="other-session"))
        self.assertAllow(self.archive([human("archive this"), self.say("done")]))

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

    # --- concurrency cap ---
    def agent_data(self, sid, **ti):
        base = {"subagent_type": "worker", "model": "sonnet", "effort": "high", "prompt": BRIEF}
        base.update(ti)
        return {"hook_event_name": "PreToolUse", "session_id": sid, "tool_name": "Agent", "tool_input": base}

    def stop_data(self, sid, **kw):
        d = {"hook_event_name": "SubagentStop", "session_id": sid, "last_assistant_message": "done"}
        d.update(kw)
        return d

    def test_cap_three_at_once_then_stop_frees_a_slot(self):
        for _ in range(3):
            self.assertAllow(self.run_guard("pretool", self.agent_data("c1")))
        res = self.run_guard("pretool", self.agent_data("c1"))
        self.assertDeny(res, "3 helpers")
        for text in ("cap is 3", "Wait for one to finish, or do the step yourself"):
            self.assertIn(text, res[2])
        # another session has its own count
        self.assertAllow(self.run_guard("pretool", self.agent_data("c2")))
        self.assertAllow(self.run_guard("subagentstop", self.stop_data("c1")))
        self.assertAllow(self.run_guard("pretool", self.agent_data("c1")))
        self.assertDeny(self.run_guard("pretool", self.agent_data("c1")), "cap is 3")

    def failure_data(self, sid, tid, **kw):
        d = {"hook_event_name": "PostToolUseFailure", "session_id": sid, "tool_name": "Agent", "tool_use_id": tid,
             "tool_input": {}, "error": "boom"}
        d.update(kw)
        return d

    def spawn_with_id(self, sid, tid):
        d = self.agent_data(sid)
        d["tool_use_id"] = tid
        return self.run_guard("pretool", d)

    def test_cap_failed_spawn_frees_its_own_slot(self):
        for tid in ("t1", "t2", "t3"):
            self.assertAllow(self.spawn_with_id("f1", tid))
        self.assertDeny(self.spawn_with_id("f1", "t4"), "cap is 3")
        # the failure of t2 frees t2's slot, not the oldest
        self.assertAllow(self.run_guard("posttoolfailure", self.failure_data("f1", "t2")))
        self.assertAllow(self.spawn_with_id("f1", "t5"))
        self.assertDeny(self.spawn_with_id("f1", "t6"), "cap is 3")
        # a failure of an unknown id, a missing id or another session frees nothing
        self.assertAllow(self.run_guard("posttoolfailure", self.failure_data("f1", "nope")))
        self.assertAllow(self.run_guard("posttoolfailure", self.failure_data("f1", None)))
        self.assertAllow(self.run_guard("posttoolfailure", self.failure_data("other", "t1")))
        self.assertDeny(self.spawn_with_id("f1", "t7"), "cap is 3")
        # freeing the same id twice frees one slot
        self.assertAllow(self.run_guard("posttoolfailure", self.failure_data("f1", "t1")))
        self.assertAllow(self.run_guard("posttoolfailure", self.failure_data("f1", "t1")))
        self.assertAllow(self.spawn_with_id("f1", "t8"))
        self.assertDeny(self.spawn_with_id("f1", "t9"), "cap is 3")

    def test_cap_failure_ignores_other_tools_and_bad_input(self):
        self.assertAllow(self.spawn_with_id("f2", "a"))
        self.assertAllow(self.run_guard("posttoolfailure", self.failure_data("f2", "a", tool_name="Bash")))
        self.assertAllow(self.run_guard("posttoolfailure", raw="not json"))
        for _ in range(2):
            self.assertAllow(self.spawn_with_id("f2", "b"))
        self.assertDeny(self.spawn_with_id("f2", "c"), "cap is 3")

    def test_cap_old_format_slots_still_load(self):
        os.makedirs(os.path.join(self.dir, "claude-guard"))
        now = time.time()
        with open(os.path.join(self.dir, "claude-guard", "f3.json"), "w") as f:
            json.dump({"spawns": [now - 30, now - 20]}, f)  # plain times from the first version
        self.assertAllow(self.spawn_with_id("f3", "n1"))
        self.assertDeny(self.spawn_with_id("f3", "n2"), "cap is 3")
        self.assertAllow(self.run_guard("subagentstop", self.stop_data("f3")))  # pops the oldest (plain) one
        self.assertAllow(self.spawn_with_id("f3", "n3"))
        self.assertAllow(self.run_guard("posttoolfailure", self.failure_data("f3", "n3")))
        self.assertAllow(self.spawn_with_id("f3", "n4"))

    def test_cap_denied_call_is_not_counted_and_stop_never_goes_negative(self):
        for _ in range(3):
            self.run_guard("pretool", self.agent_data("c3"))
        for _ in range(3):
            self.assertDeny(self.run_guard("pretool", self.agent_data("c3")))
        for _ in range(5):  # more stops than spawns
            self.assertAllow(self.run_guard("subagentstop", self.stop_data("c3")))
        for _ in range(3):
            self.assertAllow(self.run_guard("pretool", self.agent_data("c3")))
        self.assertDeny(self.run_guard("pretool", self.agent_data("c3")))

    def test_cap_other_denies_do_not_take_a_slot(self):
        for _ in range(5):  # no deadline: denied by G1 before the cap
            self.assertDeny(self.run_guard("pretool", self.agent_data("c4", prompt="build")), "a deadline")
        for _ in range(3):
            self.assertAllow(self.run_guard("pretool", self.agent_data("c4")))

    def test_cap_stop_with_denied_handback_keeps_the_slot(self):
        for _ in range(3):
            self.run_guard("pretool", self.agent_data("c5"))
        self.assertDeny(self.run_guard("subagentstop", self.stop_data("c5", last_assistant_message="x" * 5000)))
        self.assertDeny(self.run_guard("pretool", self.agent_data("c5")), "cap is 3")

    def test_cap_entries_older_than_90_minutes_are_dropped(self):
        os.makedirs(os.path.join(self.dir, "claude-guard"))
        old = time.time() - 91 * 60
        with open(os.path.join(self.dir, "claude-guard", "c6.json"), "w") as f:
            json.dump({"spawns": [old, old, old]}, f)
        for _ in range(3):
            self.assertAllow(self.run_guard("pretool", self.agent_data("c6")))
        self.assertDeny(self.run_guard("pretool", self.agent_data("c6")))

    def test_cap_by_pace(self):
        ahead = fake_pace("ahead", used=60.0, budget=50.0)
        self.assertAllow(self.inproc("pretool", self.agent_data("p1"), ahead))
        res = self.inproc("pretool", self.agent_data("p1"), ahead)
        self.assertDeny(res, "1 helper")
        self.assertIn("cap is 1", res[2])
        self.assertIn("60%", res[2])  # the pace numbers
        self.assertIn("50%", res[2])
        crit = fake_pace("critical", used=90.0, budget=100.0)
        res = self.inproc("pretool", self.agent_data("p2"), crit)
        self.assertDeny(res, "cap is 0")
        self.assertIn("90%", res[2])
        for state in ("ok", "unknown"):
            for _ in range(3):
                self.assertAllow(self.inproc("pretool", self.agent_data("p3" + state), fake_pace(state)))
            self.assertDeny(self.inproc("pretool", self.agent_data("p3" + state), fake_pace(state)), "cap is 3")

    def test_cap_without_pace_module_is_unknown(self):
        for _ in range(3):
            self.assertAllow(self.inproc("pretool", self.agent_data("p4"), None))
        self.assertDeny(self.inproc("pretool", self.agent_data("p4"), None), "cap is 3")

    def test_cap_pace_crash_is_unknown(self):
        def boom(now=None):
            raise RuntimeError("x")

        bad = types.SimpleNamespace(status=boom, refresh=boom)
        for _ in range(3):
            self.assertAllow(self.inproc("pretool", self.agent_data("p5"), bad))
        self.assertDeny(self.inproc("pretool", self.agent_data("p5"), bad), "cap is 3")

    def test_cap_parallel_calls_race_safely(self):
        if guard.fcntl is None:
            self.skipTest("no fcntl")
        results = []

        def go():
            results.append(self.run_guard("pretool", self.agent_data("par")))

        threads = [threading.Thread(target=go) for _ in range(6)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(sorted(r[0] for r in results), [0, 0, 0, 2, 2, 2])

    def test_cap_state_dir_unwritable_fails_open(self):
        bad = os.path.join(self.dir, "not-a-dir")
        with open(bad, "w") as f:
            f.write("x")
        data = self.agent_data("c9")
        for _ in range(5):
            p = subprocess.run([sys.executable, GUARD, "pretool"], input=json.dumps(data), capture_output=True,
                               text=True, env=dict(os.environ, TMPDIR=bad, HOME=self.dir), timeout=30)
            self.assertEqual((p.returncode, p.stdout, p.stderr), (0, "", ""))

    # --- plan gate ---
    def plan_transcript(self, model, entry, name="plan.jsonl"):
        """A main transcript: the entrypoint on the first record, the model on the newest assistant one."""
        first = dict(human("hi"), **({"entrypoint": entry} if entry else {}))
        rec = assistant(1000)
        if model:
            rec["message"]["model"] = model
        later = [human("go"), dict(assistant(1000), message=dict(assistant(1000)["message"], model="<synthetic>")),
                 ] if model else []
        return self.transcript([first, rec] + later, name=name)

    def skill(self, name, mode="default", pace=None, model="claude-sonnet-5-5", entry="cli", **kw):
        data = {"hook_event_name": "PreToolUse", "session_id": "g1", "tool_name": "Skill",
                "tool_input": {"skill": name}}
        if mode is not None:
            data["permission_mode"] = mode
        if model or entry:
            data["transcript_path"] = self.plan_transcript(model, entry)
        data.update(kw)
        return self.inproc("pretool", data, pace) if pace is not None else self.run_guard("pretool", data)

    def test_plan_gate_denies_planning_skills_outside_plan_mode(self):
        # CLI + Sonnet or Haiku: opusplan applies, so plans go through plan mode
        for name in ("superpowers:brainstorming", "superpowers:writing-plans"):
            for model in ("claude-sonnet-5-5", "claude-haiku-4-5"):
                for pace in (None, fake_pace("ok"), fake_pace("unknown")):
                    res = self.skill(name, pace=pace, model=model)
                    self.assertDeny(res, "EnterPlanMode")
                    self.assertIn("plan mode", res[2])
                    self.assertIn("(autopilot: send a `Plan` helper instead)", res[2])
            self.assertDeny(self.skill(name, mode=None), "EnterPlanMode")
            self.assertDeny(self.skill(name, mode="acceptEdits"), "EnterPlanMode")

    def test_plan_gate_desktop_sonnet_points_to_plan_helper_or_opus(self):
        for name in ("superpowers:brainstorming", "superpowers:writing-plans"):
            res = self.skill(name, entry="claude-desktop")
            self.assertDeny(res, "send a `Plan` helper (Opus by the router), or pick Opus in the model menu")
            self.assertNotIn("EnterPlanMode", res[2])
            self.assertDeny(self.skill(name, entry="claude-desktop", pace=fake_pace("ok")), "pick Opus")
            self.assertAllow(self.skill(name, entry="claude-desktop", mode="plan"))

    def test_plan_gate_allows_opus_and_fable_mains(self):
        for model in ("claude-opus-5-5", "claude-fable-5-1", "opus"):
            for entry in ("cli", "claude-desktop"):
                self.assertAllow(self.skill("superpowers:writing-plans", model=model, entry=entry))
                self.assertAllow(self.skill("superpowers:brainstorming", model=model, entry=entry,
                                            pace=fake_pace("ok")))

    def test_plan_gate_unknown_model_or_entrypoint_allows(self):
        name = "superpowers:writing-plans"
        self.assertAllow(self.skill(name, model=None, entry=None))  # no transcript at all
        self.assertAllow(self.skill(name, model=None, entry="cli"))  # no assistant record
        self.assertAllow(self.skill(name, model="claude-sonnet-5-5", entry=None))  # no entrypoint
        self.assertAllow(self.skill(name, model="claude-sonnet-5-5", entry="sdk-cli"))  # not cli or desktop
        self.assertAllow(self.skill(name, model="mystery-model", entry="cli"))
        self.assertAllow(self.skill(name, transcript_path="/no/such/file.jsonl"))

    def test_plan_gate_uses_the_newest_assistant_model(self):
        path = self.transcript([dict(human("hi"), entrypoint="cli"),
                                dict(assistant(1000), message=dict(assistant(1000)["message"], model="claude-sonnet-5-5")),
                                dict(assistant(1000), message=dict(assistant(1000)["message"], model="claude-opus-5-5"))])
        data = {"hook_event_name": "PreToolUse", "session_id": "g1", "tool_name": "Skill", "permission_mode": "default",
                "tool_input": {"skill": "superpowers:writing-plans"}, "transcript_path": path}
        self.assertAllow(self.run_guard("pretool", data))  # switched to Opus mid-session

    def test_plan_gate_allows(self):
        self.assertAllow(self.skill("superpowers:writing-plans", mode="plan"))
        self.assertAllow(self.skill("superpowers:brainstorming", mode="plan"))
        self.assertAllow(self.skill("superpowers:test-driven-development"))
        self.assertAllow(self.skill("ps:handoff"))
        self.assertAllow(self.skill("superpowers:writing-plans", agent_id="a1"))  # a subagent
        self.assertAllow(self.skill("superpowers:writing-plans", transcript_path="/x/subagents/agent-1.jsonl"))
        for state in ("ahead", "critical"):  # Sonnet plans without plan mode
            self.assertAllow(self.skill("superpowers:writing-plans", pace=fake_pace(state, 90.0, 50.0)))
            self.assertAllow(self.skill("superpowers:brainstorming", pace=fake_pace(state, 90.0, 50.0)))

    def test_enter_plan_mode_denied_when_ahead(self):
        def enter(pace=None, **kw):
            data = {"hook_event_name": "PreToolUse", "session_id": "g2", "tool_name": "EnterPlanMode",
                    "tool_input": {}}
            data.update(kw)
            return self.inproc("pretool", data, pace) if pace is not None else self.run_guard("pretool", data)

        for state in ("ahead", "critical"):
            res = enter(fake_pace(state, 90.0, 50.0))
            self.assertDeny(res, "Plan without plan mode")
            self.assertIn("90%", res[2])
        self.assertAllow(enter(fake_pace("ok")))
        self.assertAllow(enter(fake_pace("unknown")))
        self.assertAllow(enter())

    # --- pace notes (UserPromptSubmit) ---
    def prompt_pace(self, pace, sid="n1", ctx=1000):
        path = self.transcript([human(), assistant(ctx)], name="%s.jsonl" % sid)
        data = {"hook_event_name": "UserPromptSubmit", "session_id": sid, "transcript_path": path, "prompt": "go"}
        rc, out, err = self.inproc("prompt", data, pace)
        self.assertEqual((rc, err), (0, ""))
        return json.loads(out)["hookSpecificOutput"]["additionalContext"] if out else None

    def test_pace_note_once_when_ahead(self):
        calls = []
        ahead = fake_pace("ahead", used=62.0, budget=50.0, calls=calls)
        note = self.prompt_pace(ahead)
        for text in ("62%", "50%", "Sonnet", "max 1 at once", "without plan mode", "pick Sonnet"):
            self.assertIn(text, note)
        self.assertIsNone(self.prompt_pace(ahead))  # only once
        self.assertEqual(calls, [5, 5])  # refresh(budget_s=5) on every prompt
        self.assertIsNone(self.prompt_pace(fake_pace("unknown")))
        self.assertIsNone(self.prompt_pace(ahead))  # unknown does not reset the memory

    def test_pace_note_critical_and_return_to_ok(self):
        self.assertIsNone(self.prompt_pace(fake_pace("ok")))
        self.assertIn("Sonnet", self.prompt_pace(fake_pace("ahead", 60.0, 50.0)))
        note = self.prompt_pace(fake_pace("critical", 90.0, 100.0))
        for text in ("90%", "Sonnet only", "no helpers", "Tell the user"):
            self.assertIn(text, note)
        # critical repeats on every prompt, ahead does not
        for used in (91.0, 92.0):
            self.assertIn("Sonnet only", self.prompt_pace(fake_pace("critical", used, 100.0)))
        self.assertIsNone(self.prompt_pace(fake_pace("ok")))
        self.assertIn("Sonnet", self.prompt_pace(fake_pace("ahead", 60.0, 50.0)))  # ahead again after ok

    def test_pace_note_sessions_independent_and_joined_with_nudge(self):
        ahead = fake_pace("ahead", 60.0, 50.0)
        self.assertIsNotNone(self.prompt_pace(ahead, sid="n2"))
        self.assertIsNotNone(self.prompt_pace(ahead, sid="n3"))
        both = self.prompt_pace(fake_pace("critical", 90.0, 100.0), sid="n4", ctx=350_000)
        self.assertIn("Context is 350K", both)
        self.assertIn("Sonnet only", both)

    def test_pace_note_fail_open(self):
        def boom(*a, **k):
            raise RuntimeError("x")

        self.assertIsNone(self.prompt_pace(types.SimpleNamespace(status=boom, refresh=boom), sid="n5"))
        self.assertIsNone(self.prompt_pace(None, sid="n6"))
        # a refresh that fails does not stop the status read
        bad_refresh = types.SimpleNamespace(
            status=fake_pace("ahead", 60.0, 50.0).status, refresh=boom)
        self.assertIn("Sonnet", self.prompt_pace(bad_refresh, sid="n7"))

    # --- off switch ---
    def test_check_off(self):
        with mock.patch.dict(os.environ, {"GUARD_OFF": "g10, G13"}):
            self.assertTrue(guard.check_off("G10"))
            self.assertTrue(guard.check_off("G13"))
            self.assertFalse(guard.check_off("G1"))
        with mock.patch.dict(os.environ, {"GUARD_OFF": ""}):
            self.assertFalse(guard.check_off("G10"))

    # --- G10 to G12 Stop checks and the G6 fix (the guard-stop helper adds its tests here) ---
    def stamp(self, ago_s):
        return time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(time.time() - ago_s))

    def human_at(self, ago_s, text="go"):
        return dict(human(text), timestamp=self.stamp(ago_s))

    def tu(self, name, tool_input, tid="u1"):
        return assistant(1000, [{"type": "tool_use", "id": tid, "name": name, "input": tool_input}])

    def stop_msg(self, recs, env=None, **extra):
        with mock.patch.dict(os.environ, env or {}):
            return self.stop_run(recs, **extra)

    # G6 fix: a hand-off that is gone from disk is not owed to the user
    def test_g6_deleted_handoff_is_skipped(self):
        gone = os.path.join(self.dir, "handoffs", "gone.md")
        base = [human("hand off"), self.write_ho(path=gone), tool_result("w1"), self.say("written")]
        self.assertAllow(self.stop_run(base))
        self.assertAllow(self.archive(base))
        os.makedirs(os.path.dirname(gone))
        with open(gone, "w") as f:
            f.write("# Task")
        self.assertDeny(self.stop_run(base), "Read " + gone)
        self.assertDeny(self.archive(base), "Do not archive yet")

    # G10 commit check
    def git(self, repo, *args):
        env = dict(os.environ, HOME=self.dir, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        r = subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
                            "-C", repo] + list(args), capture_output=True, text=True, env=env, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def make_repo(self, upstream=True):
        work = os.path.join(os.path.realpath(self.dir), "work")
        os.makedirs(work)
        self.git(work, "init", "-q")
        self.git(work, "symbolic-ref", "HEAD", "refs/heads/main")
        self.write_file(work, "base.txt", "base")
        self.git(work, "add", "-A")
        self.git(work, "commit", "-q", "-m", "base")
        if upstream:
            bare = os.path.join(os.path.realpath(self.dir), "origin.git")
            os.makedirs(bare)
            self.git(bare, "init", "-q", "--bare")
            self.git(work, "remote", "add", "origin", bare)
            self.git(work, "push", "-q", "-u", "origin", "main")
        return work

    def write_file(self, repo, name, text="x"):
        p = os.path.join(repo, name)
        with open(p, "w") as f:
            f.write(text)
        return p

    def g10(self, recs, repo, **extra):
        return self.stop_msg(recs, cwd=repo, **extra)

    def test_g10_dirty_file_touched_this_turn(self):
        repo = self.make_repo()
        p = self.write_file(repo, "new.txt")
        recs = [human("build"), self.tu("Write", {"file_path": p}), tool_result("u1"), self.say("done")]
        res = self.g10(recs, repo)
        self.assertDeny(res, "G10: this turn left work in work: 1 uncommitted file(s), 0 unpushed commit(s).")
        self.assertIn("Rule 13: commit and push now, unless the user said not to.", res[2])
        # Edit and NotebookEdit count as well
        q = self.write_file(repo, "base.txt", "changed")
        self.assertDeny(self.g10([human("x"), self.tu("Edit", {"file_path": q})], repo), "1 uncommitted")
        n = self.write_file(repo, "nb.ipynb", "{}")
        self.assertDeny(self.g10([human("x"), self.tu("NotebookEdit", {"notebook_path": n})], repo), "1 uncommitted")

    def test_g10_untouched_files_stay_quiet(self):
        repo = self.make_repo()
        self.write_file(repo, "other.txt")  # dirty, but the session never wrote it
        p = self.write_file(repo, "mine.txt")
        self.git(repo, "add", "mine.txt")
        self.git(repo, "commit", "-q", "-m", "mine")
        self.git(repo, "push", "-q")
        self.assertAllow(self.g10([human("x"), self.tu("Write", {"file_path": p}), self.say("ok")], repo))
        # touched in an earlier turn only
        d = self.write_file(repo, "old.txt")
        recs = [human("a"), self.tu("Write", {"file_path": d}), human("b"), self.say("chat")]
        self.assertAllow(self.g10(recs, repo))
        # a file outside the repo is not in its status
        outside = self.write_file(self.dir, "elsewhere.txt")
        self.assertAllow(self.g10([human("x"), self.tu("Write", {"file_path": outside})], repo))

    def test_g10_unpushed_needs_a_commit_this_turn(self):
        repo = self.make_repo()
        p = self.write_file(repo, "a.txt")
        self.git(repo, "add", "-A")
        self.git(repo, "commit", "-q", "-m", "a")
        commit = self.tu("Bash", {"command": 'git commit -m "a"'})
        res = self.g10([human("x"), self.tu("Write", {"file_path": p}), commit, self.say("ok")], repo)
        self.assertDeny(res, "0 uncommitted file(s), 1 unpushed commit(s).")
        # an unpushed commit from an earlier turn is not this turn's work
        self.assertAllow(self.g10([human("x"), commit, human("y"), self.say("chat")], repo))
        self.assertAllow(self.g10([human("x"), self.say("chat")], repo))

    def test_g10_user_said_not_to_push(self):
        repo = self.make_repo()
        p = self.write_file(repo, "new.txt")
        work = [self.tu("Write", {"file_path": p}), self.say("done")]
        for phrase in ("do not push", "Don't push please", "NO PUSH", "keep it local only", "don't commit", "Do Not Commit"):
            recs = [human("first. " + phrase), human("more"), ] + work
            self.assertAllow(self.g10(recs, repo))
        recs = [{"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "local only"}]}}]
        self.assertAllow(self.g10(recs + [human("go")] + work, repo))
        self.assertDeny(self.g10([human("please push when done")] + work, repo), "G10")

    def test_g10_no_upstream_and_git_failures_are_quiet(self):
        repo = self.make_repo(upstream=False)
        p = self.write_file(repo, "new.txt")
        recs = [human("x"), self.tu("Write", {"file_path": p}), self.say("done")]
        # no upstream: unpushed counts as 0, but a dirty file of this turn still fires
        self.assertDeny(self.g10(recs, repo), "G10: this turn left work in work: 1 uncommitted file(s), 0 unpushed commit(s).")
        self.git(repo, "add", "-A")
        self.git(repo, "commit", "-q", "-m", "new")
        self.assertAllow(self.g10(recs, repo))  # clean, and nothing to compare with
        commit = self.tu("Bash", {"command": 'git commit -m "new"'})
        self.assertAllow(self.g10([human("x"), commit, self.say("done")], repo))
        plain = os.path.join(self.dir, "plain")
        os.makedirs(plain)
        self.assertAllow(self.g10(recs, plain))  # not a repo
        self.assertAllow(self.g10(recs, os.path.join(self.dir, "missing")))
        self.assertAllow(self.stop_run(recs))  # no cwd at all

    def test_g10_git_timeout_or_crash_fails_open(self):
        repo = self.make_repo()
        p = self.write_file(repo, "new.txt")
        path = self.transcript([human("x"), self.tu("Write", {"file_path": p}), self.say("done")])
        data = {"transcript_path": path, "cwd": repo}
        self.assertTrue(guard.stop(dict(data)))  # sanity: fires with a working git
        for exc in (subprocess.TimeoutExpired("git", 2), OSError("no git"), ValueError("boom")):
            with mock.patch.object(guard.subprocess, "run", side_effect=exc):
                self.assertIsNone(guard.stop(dict(data)))

    def test_g10_switches(self):
        repo = self.make_repo()
        p = self.write_file(repo, "new.txt")
        recs = [human("x"), self.tu("Write", {"file_path": p}), self.say("done")]
        self.assertDeny(self.g10(recs, repo), "G10")
        self.assertAllow(self.g10(recs, repo, stop_hook_active=True))
        self.assertAllow(self.stop_msg(recs, env={"GUARD_OFF": "G10"}, cwd=repo))
        self.assertAllow(self.g10(recs, repo, agent_id="a1"))
        self.assertAllow(self.run_guard("stop", {"hook_event_name": "Stop", "cwd": repo,
                                                 "transcript_path": os.path.join(self.dir, "missing.jsonl")}))

    # G11 clock check
    def test_g11_long_turn_without_a_clock(self):
        recs = [self.human_at(12 * 60 + 10), self.say("All done.")]
        res = self.stop_run(recs)
        self.assertDeny(res, "G11: this turn ran 12 min. Rule 12: give the clock as a Vienna time (HH:MM) "
                             "and one estimate with a range.")
        self.assertAllow(self.stop_run([self.human_at(5 * 60), self.say("All done.")]))
        self.assertAllow(self.stop_run([human("no stamp"), self.say("All done.")]))

    def test_g11_a_clock_or_duration_in_the_reply_is_enough(self):
        base = [self.human_at(20 * 60)]
        for reply in ("Done at 14:05.", "That took 20 min.", "About 2 hours left.", "Roughly 1 h more.",
                      "Estimate: 15-30 minutes."):
            with self.subTest(reply=reply):
                self.assertAllow(self.stop_run(base + [self.say(reply)]))
                self.assertAllow(self.stop_run(base, last_assistant_message=reply))
        self.assertDeny(self.stop_run(base + [self.say("Done at 14:05.")], last_assistant_message="Done."), "G11")
        self.assertDeny(self.stop_run(base + [self.say("Version 3 is out, 5 files changed.")]), "G11")

    def test_g11_switches_and_missing_transcript(self):
        recs = [self.human_at(15 * 60), self.say("Done.")]
        self.assertAllow(self.stop_run(recs, stop_hook_active=True))
        self.assertAllow(self.stop_msg(recs, env={"GUARD_OFF": "G11"}))
        self.assertAllow(self.stop_run(recs, agent_id="a1"))
        self.assertAllow(self.run_guard("stop", {"hook_event_name": "Stop",
                                                 "transcript_path": os.path.join(self.dir, "missing.jsonl")}))
        # the turn start is the LAST human prompt, not an earlier one
        self.assertAllow(self.stop_run([self.human_at(60 * 60), self.say("a"), self.human_at(60), self.say("b")]))
        # a tool_result is not a prompt
        self.assertDeny(self.stop_run([self.human_at(15 * 60), tool_result("t1"), self.say("b")]), "G11")

    # G12 open steps check
    def plan(self, **kw):
        return self.tu("mcp__ccd_turn__update_plan", kw, tid="p%d" % len(kw))

    def items(self, *specs):
        return [{"content": c, "status": s} for c, s in specs]

    def test_g12_open_steps_fire(self):
        recs = [human("go"), self.plan(items=self.items(("Read brief", "in_progress"), ("Build", "pending"),
                                                         ("Test", "pending"), ("Ship", "pending"))),
                self.say("Working on it.")]
        self.assertDeny(self.stop_run(recs), "G12: the task list still has open steps (Read brief, Build, Test). "
                                             "Finish them now, or end with what is done, what is left, and one question.")

    def test_g12_replays_advance_done_and_start(self):
        base = [human("go"), self.plan(items=self.items(("A", "in_progress"), ("B", "pending"), ("C", "pending")))]
        self.assertDeny(self.stop_run(base + [self.plan(advance=True), self.say("x")]), "steps (B, C)")
        done = base + [self.plan(advance=True), self.plan(advance=True), self.plan(advance=True), self.say("x")]
        self.assertAllow(self.stop_run(done))
        self.assertDeny(self.stop_run(base + [self.plan(done=[1]), self.say("x")]), "steps (B, C)")
        self.assertDeny(self.stop_run(base + [self.plan(done=[1, 3]), self.say("x")]), "steps (B)")
        self.assertAllow(self.stop_run(base + [self.plan(done=[1, 2, 3]), self.say("x")]))
        self.assertDeny(self.stop_run(base + [self.plan(done=[1], start=3), self.say("x")]), "steps (B, C)")
        self.assertDeny(self.stop_run(base + [self.plan(done=[1, 2], start=3), self.say("x")]), "steps (C)")
        # an items call replaces the list
        again = base + [self.plan(advance=True), self.plan(items=self.items(("Z", "pending"))), self.say("x")]
        self.assertDeny(self.stop_run(again), "steps (Z)")
        shut = base + [self.plan(items=self.items(("Z", "completed"), ("Y", "done"))), self.say("x")]
        self.assertAllow(self.stop_run(shut))

    def test_g12_list_from_an_earlier_turn_still_counts(self):
        recs = [human("go"), self.plan(items=self.items(("A", "in_progress"), ("B", "pending"))), self.say("part"),
                human("continue"), self.say("hm")]
        self.assertDeny(self.stop_run(recs), "steps (A, B)")

    def test_g12_stays_quiet_when_asked_wrapped_or_questioned(self):
        lst = self.plan(items=self.items(("A", "in_progress"), ("B", "pending")))
        self.assertAllow(self.stop_run([human("go"), lst, self.tu("AskUserQuestion", {"questions": ["q"]}, tid="q1"),
                                        self.say("Waiting.")]))
        self.assertAllow(self.stop_run([human("go"), lst, self.tu("mcp__ccd_turn__wrap_up", {"recap": "r"}, tid="w1"),
                                        self.say("Done.")]))
        self.assertAllow(self.stop_run([human("go"), lst, self.say("Done A. B is left. Shall I go on?")]))
        self.assertAllow(self.stop_run([human("go"), lst, self.say("x")], last_assistant_message="Shall I go on?  "))
        # a wrap_up after the last update_plan closes every step, also from an earlier turn
        wrapped = [human("go"), lst, self.tu("mcp__ccd_turn__wrap_up", {"recap": "r"}, tid="w1"), self.say("ok"),
                   human("thanks"), self.say("welcome")]
        self.assertAllow(self.stop_run(wrapped))
        # but a later update_plan reopens the list
        reopen = wrapped + [self.plan(items=self.items(("C", "pending"))), self.say("x")]
        self.assertDeny(self.stop_run(reopen), "steps (C)")
        # an AskUserQuestion from an earlier turn does not count
        old = [human("go"), self.tu("AskUserQuestion", {"questions": ["q"]}, tid="q1"), lst, human("ok"), self.say("x")]
        self.assertDeny(self.stop_run(old), "G12")
        self.assertAllow(self.stop_run([human("go"), self.say("no plan at all")]))

    def test_g12_quiet_while_a_helper_slot_is_live(self):
        recs = [human("go"), self.plan(items=self.items(("A", "pending"))), self.say("x")]
        self.assertDeny(self.stop_run(recs), "G12")  # no slot: reminder
        state = os.path.join(self.dir, "claude-guard")
        os.makedirs(state, exist_ok=True)
        with open(os.path.join(state, "s.json"), "w") as f:
            json.dump({"spawns": [{"id": "a1", "t": time.time() - 30}]}, f)
        self.assertAllow(self.stop_run(recs))  # live slot: a helper still runs
        with open(os.path.join(state, "s.json"), "w") as f:
            json.dump({"spawns": [{"id": "a1", "t": time.time() - 10 * guard.SPAWN_TTL}]}, f)
        self.assertDeny(self.stop_run(recs), "G12")  # expired slot does not count
        with open(os.path.join(state, "s.json"), "w") as f:
            f.write("not json {{{")
        self.assertDeny(self.stop_run(recs), "G12")  # broken state file = no slot

    def test_g12_switches_and_missing_transcript(self):
        recs = [human("go"), self.plan(items=self.items(("A", "pending"))), self.say("x")]
        self.assertDeny(self.stop_run(recs), "G12")
        self.assertAllow(self.stop_run(recs, stop_hook_active=True))
        self.assertAllow(self.stop_msg(recs, env={"GUARD_OFF": "G12"}))
        self.assertAllow(self.stop_run(recs, agent_id="a1"))
        self.assertAllow(self.run_guard("stop", {"hook_event_name": "Stop",
                                                 "transcript_path": os.path.join(self.dir, "missing.jsonl")}))

    def test_stop_checks_run_in_order_and_only_one_fires(self):
        repo = self.make_repo()
        p = self.write_file(repo, "new.txt")
        recs = [self.human_at(15 * 60), self.plan(items=self.items(("A", "pending"))),
                self.tu("Write", {"file_path": p}), self.say("x")]
        self.assertDeny(self.g10(recs, repo), "G10")  # G10 first
        self.assertDeny(self.stop_run(recs), "G11")  # then G11
        recs[0] = human("go")
        self.assertDeny(self.stop_run(recs), "G12")  # then G12

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
        d = {"tool_name": "Agent", "tool_input": {"model": "sonnet", "effort": "high", "prompt": BRIEF, "subagent_type": "worker"},
             "transcript_path": "/no/such"}
        self.assertAllow(self.run_guard("pretool", d))
        d = {"tool_name": "ScheduleWakeup", "tool_input": {"delaySeconds": 60}, "transcript_path": "/no/such"}
        self.assertAllow(self.run_guard("pretool", d))

    # --- G13 to G15 tool checks and the G3 text (the guard-tools helper adds its tests here) ---
    # G3 text
    def test_g3_text_no_handoff_push(self):
        rc, out, err = self.prompt_run(350_000, sid="g3t")
        note = json.loads(out)["hookSpecificOutput"]["additionalContext"]
        self.assertEqual(
            note,
            "Context is 350K. Keep PROGRESS.md (or the run's ledger) current; auto-compact carries the "
            "run. Hand off only when the user asks.",
        )
        ok = {"model": "sonnet", "effort": "high", "prompt": BRIEF}
        res = self.pre("Agent", ok, 600_000)
        self.assertDeny(res, "context is 600K (> 350K)")
        self.assertNotIn("handoff", res[2])
        self.assertNotIn("fresh session", res[2])

    # G14 sed -i
    def g14(self, cmd, platform="darwin", env=None):
        data = {"tool_name": "Bash", "tool_input": {"command": cmd}}
        with mock.patch.object(sys, "platform", platform), mock.patch.dict(os.environ, env or {}):
            return guard.g14_sed(data, None)

    G14_MSG = (
        "G14: BSD sed -i on macOS differs from GNU sed and breaks files. Use the Edit tool, or "
        "python3 for bulk edits."
    )

    def test_g14_denies_in_place_forms(self):
        for cmd in (
            "sed -i 's/a/b/' f",
            "sed -i '' 's/a/b/' f",
            "sed -i.bak s/a/b/ f",
            "sed -ni 's/a/b/p' f",
            "sed -Ei 's/a/b/' f",
            "sed -i -E 's/a/b/' f",
            "sed --in-place s/a/b/ f",
            "sed --in-place=.bak s/a/b/ f",
            "sed -n -i p f",
            "sed -e s/a/b/ -i f",
            "sed s/a/b/ -i f",
            "/usr/bin/sed -i s/a/b/ f",
            "FOO=1 sed -i s/a/b/ f",
        ):
            self.assertEqual(self.g14(cmd), self.G14_MSG, cmd)

    def test_g14_denies_after_separators_and_wrappers(self):
        for cmd in (
            "cd x && sed -i s/a/b/ f",
            "echo a; sed -i s/a/b/ f",
            "false || sed -i s/a/b/ f",
            "ls | sed -i s/a/b/",
            "echo f | xargs sed -i s/a/b/",
            "echo f | xargs -I {} sed -i s/a/b/ {}",
            "find . -name '*.md' -exec sed -i '' s/a/b/ {} \\;",
            "find . -name '*.md' -exec sed -i '' s/a/b/ {} +",
            "sudo sed -i s/a/b/ f",
            "sudo -u root sed -i s/a/b/ f",
            "env FOO=1 sed -i s/a/b/ f",
            "env -i sed -i s/a/b/ f",
            'bash -c "sed -i s/a/b/ f"',
            "echo a\nsed -i s/a/b/ f",
            "(cd x; sed -i s/a/b/ f)",
        ):
            self.assertEqual(self.g14(cmd), self.G14_MSG, cmd)

    def test_g14_allows(self):
        for cmd in (
            "sed -n '1,5p' f",
            "sed -E 's/a/b/' f > g",
            "sed -En 's/a/b/p' f",
            "sed -e 's/-i/x/' f",
            "sed -s -n p f",
            "sed 's/a/b/' f",
            "sed -n -e 'p' f",
            "gsed -i s/a/b/ f",
            'echo "sed -i"',
            "rg 'sed -i' f",
            "grep sed -i f",
            "ls -i",
            'git commit -m "no more sed -i here"',
            "cat <<'EOF'\nsed -i foo\nEOF",
            "echo x | tee out.txt",
            "",
        ):
            self.assertIsNone(self.g14(cmd), cmd)

    def test_g14_macos_only_off_switch_and_fail_open(self):
        self.assertIsNone(self.g14("sed -i s/a/b/ f", platform="linux"))
        self.assertIsNone(self.g14("sed -i s/a/b/ f", env={"GUARD_OFF": "G14"}))
        self.assertEqual(self.g14("sed -i s/a/b/ f", env={"GUARD_OFF": "G13"}), self.G14_MSG)
        with mock.patch.object(sys, "platform", "darwin"):
            for data in (
                {"tool_name": "Bash"},
                {"tool_name": "Bash", "tool_input": "oops"},
                {"tool_name": "Bash", "tool_input": {"command": None}},
                {"tool_name": "Bash", "tool_input": {"command": ["sed", "-i"]}},
                {"tool_name": "Bash", "tool_input": {"command": "echo 'unbalanced sed -i"}},
                {"tool_name": "Edit", "tool_input": {"command": "sed -i x"}},
            ):
                self.assertIsNone(guard.g14_sed(data, None), data)

    def test_g14_through_pretool(self):
        if sys.platform != "darwin":
            self.skipTest("G14 is macOS only")
        self.assertDeny(self.pre("Bash", {"command": "sed -i '' s/a/b/ f"}), "G14: BSD sed -i")
        self.assertAllow(self.pre("Bash", {"command": "sed -n 1p f"}))

    # G15 secret reads
    G15_FILE = (
        "G15: never read local.zsh: it holds API keys. Use `test -f` or `git check-ignore` for facts "
        "about it, or ask the user."
    )
    G15_SEARCH = (
        "G15: this search can reach local.zsh (API keys). Exclude it: `rg -g '!local.zsh' ...` or "
        "`grep --exclude=local.zsh ...`."
    )
    G15_PS = (
        "G15: this prints full command lines, which can hold API keys. Use `pgrep -f <name>` for pids, "
        "or `ps -o pid=,etime=,ucomm= -p <pid>`."
    )

    def g15(self, tool, ti, env=None, platform=None, **extra):
        data = dict({"tool_name": tool, "tool_input": ti}, **extra)
        env = dict({"HOME": self.dir}, **(env or {}))
        with mock.patch.dict(os.environ, env), mock.patch.object(sys, "platform", platform or sys.platform):
            return guard.g15_secrets(data, None)

    def g15bash(self, cmd, **kw):
        return self.g15("Bash", {"command": cmd}, **kw)

    def test_g15_read_tool(self):
        zsh = os.path.join(self.dir, ".config", "zsh")
        os.makedirs(zsh)
        target = os.path.join(zsh, "local.zsh")
        with open(target, "w") as f:
            f.write("export KEY=1\n")
        link = os.path.join(self.dir, "link.zsh")
        os.symlink(target, link)
        for path in ("~/.config/zsh/local.zsh", target, link, os.path.join(zsh, "..", "zsh", "local.zsh")):
            self.assertEqual(self.g15("Read", {"file_path": path}), self.G15_FILE, path)
        for path in ("~/.config/zsh/aliases.zsh", os.path.join(self.dir, "README.md"), "/etc/hosts", ""):
            self.assertIsNone(self.g15("Read", {"file_path": path}), path)
        self.assertEqual(self.g15("Read", {"file_path": "local.zsh"}, cwd=zsh), self.G15_FILE)
        self.assertIsNone(self.g15("Read", {"file_path": "local.zsh"}, cwd=self.dir))

    def test_g15_bash_readers_denied(self):
        for cmd in (
            "cat ~/.config/zsh/local.zsh",
            'cat "$HOME/.config/zsh/local.zsh"',
            "cat ${HOME}/.config/zsh/local.zsh",
            "cat ~/.config/zsh/../zsh/local.zsh",
            "bat ~/.config/zsh/local.zsh",
            "less ~/.config/zsh/local.zsh",
            "head -5 ~/.config/zsh/local.zsh",
            "tail -n 3 ~/.config/zsh/local.zsh",
            "rg KEY ~/.config/zsh/local.zsh",
            "grep -n KEY ~/.config/zsh/local.zsh",
            "sed -n 1p ~/.config/zsh/local.zsh",
            "awk '{print}' ~/.config/zsh/local.zsh",
            "cut -d= -f2 ~/.config/zsh/local.zsh",
            "strings ~/.config/zsh/local.zsh",
            "xxd ~/.config/zsh/local.zsh",
            "cp ~/.config/zsh/local.zsh /tmp/x",
            "open ~/.config/zsh/local.zsh",
            "cd ~ && cat .config/zsh/local.zsh",
            "cd ~/.config/zsh && cat local.zsh",
            "cat ~/.config/zsh/local.z*",
            "echo x | cat - ~/.config/zsh/local.zsh",
            "xargs cat < ~/.config/zsh/local.zsh",
            "while read l; do echo $l; done < ~/.config/zsh/local.zsh",
            "find ~/.config/zsh -name local.zsh -exec cat {} \\;",
            "sudo cat ~/.config/zsh/local.zsh",
        ):
            self.assertEqual(self.g15bash(cmd), self.G15_FILE, cmd)
        self.assertEqual(self.g15bash("rg KEY ~/.config/zsh"), self.G15_SEARCH)

    def test_g15_bash_allowed(self):
        for cmd in (
            "ls ~/.config/zsh/local.zsh",
            "ls -la ~/.config/zsh",
            "test -f ~/.config/zsh/local.zsh",
            "[ -f ~/.config/zsh/local.zsh ] && echo yes",
            "stat ~/.config/zsh/local.zsh",
            "cat ~/.config/zsh/aliases.zsh",
            "cat README.md",
            "rg KEY ~/dotfiles",
            "head -5 /tmp/local.txt",
            'echo "cat ~/.config/zsh/local.zsh"',
            "rg 'local.zsh' ~/dotfiles/docs",
            "git add home/.config/zsh/aliases.zsh",
            "echo hi >> ~/.config/zsh/local.zsh",
            "",
        ):
            self.assertIsNone(self.g15bash(cmd), cmd)

    def test_g15_process_command_lines_denied(self):
        for cmd in (
            "ps aux",
            "ps auxww",
            "ps aux | grep node",
            "ps -ef",
            "ps -ef | grep x",
            "ps ax",
            "ps -e",
            "ps -A",
            "ps -f -p 12",
            "ps -o args",
            "ps -o command -p 1",
            "ps -p 1 -o pid,command",
            "ps -p 1 -o pid,args=",
            "ps -p 1 -opid,args",
            "ps -axo pid,command",
            "ps -eo pid,args",
            "ps --format pid,args",
            "ps -ww",
            "ps -p 1 -ww",
            "ps eww",
            "ps eww -p 1",
            "/bin/ps aux",
            "sudo ps aux",
            "pgrep -fl node",
            "pgrep -af node",
            "pgrep -lf node",
            "pgrep -f -l node",
            "pgrep -f -a node",
            "pgrep --list-full node",
            "procs",
            "procs --tree",
            "echo x; procs",
        ):
            self.assertEqual(self.g15bash(cmd), self.G15_PS, cmd)

    def test_g15_process_checks_allowed(self):
        for cmd in (
            "pgrep -f node",
            "pgrep node",
            "pgrep -l node",
            "pgrep -u me -f node",
            "pkill -f node",
            "ps -p 123 -o pid,etime,stat,comm",
            "ps -o pid,etime,comm -p 123",
            "ps -p 123 -o etime=",
            "ps -axo pid,comm",
            "ps -e -o pid,comm",
            "ps -p 123",
            "ps -u me -o pid,comm",
            "ps --help",
            'echo "ps aux"',
            "rg 'ps aux' docs",
            "kill -0 123",
            "lsof -p 123",
        ):
            self.assertIsNone(self.g15bash(cmd, platform="linux"), cmd)  # Linux behaviour is unchanged

    def secret_tree(self):
        """A temp HOME whose ~/.config/zsh links into a dotfiles repo, like the real machine. Returns the repo."""
        repo = os.path.join(os.path.realpath(self.dir), "dotfiles")
        zsh = os.path.join(repo, "home", ".config", "zsh")
        os.makedirs(zsh)
        for name in ("local.zsh", "aliases.zsh"):
            with open(os.path.join(zsh, name), "w") as f:
                f.write("export KEY=1\n")
        os.makedirs(os.path.join(self.dir, ".config"))
        os.symlink(zsh, os.path.join(self.dir, ".config", "zsh"))
        return repo

    def test_g15_recursive_searches_denied(self):
        repo = self.secret_tree()
        for cmd in (
            "grep -rn KEY .",
            "grep -R KEY",
            "grep --recursive KEY .",
            "grep -d recurse KEY .",
            "grep -drecurse KEY .",
            "grep --directories=recurse KEY .",
            "grep -r -e KEY .",
            "grep -rn KEY home",
            "egrep -rn KEY .",
            "fgrep -r KEY ~",
            "grep -rn KEY ~/dotfiles",
            "grep -rl KEY ..",
            "grep -r --exclude=other.txt KEY .",
            "grep -r --exclude-dir=docs KEY .",
            "ack KEY",
            "ack KEY .",
            "rg -u KEY .",
            "rg -uu KEY",
            "rg -uuu KEY .",
            "rg -nu KEY .",
            "rg --no-ignore KEY .",
            "rg --no-ignore-vcs KEY",
            "rg --hidden KEY .",
            "rg -. KEY .",
            "rg -n --hidden KEY home",
            "rg -u -g '!other' KEY .",
            "ag -u KEY",
            "ag --unrestricted KEY",
            "ag -U KEY .",
            "ag --skip-vcs-ignores KEY .",
            "ag --hidden KEY .",
            "sudo rg -u KEY .",
            "cd .. && grep -rn KEY dotfiles",
        ):
            self.assertEqual(self.g15bash(cmd, cwd=repo), self.G15_SEARCH, cmd)

    def test_g15_searches_allowed(self):
        repo = self.secret_tree()
        for cmd in (
            "rg KEY .",
            "rg -n x claude/",
            "rg KEY ~",
            "rg -u KEY claude/",
            "rg --hidden -n x claude/",
            "ag KEY .",
            "grep -n x file",
            "grep KEY .",
            "grep -r KEY claude/",
            "grep -rn x claude/hooks",
            "grep -r --include='*.py' KEY .",
            "sed -n 1,5p f",
            "ps -o pid=,ucomm= -p 1",
            "test -f ~/.config/zsh/local.zsh",
            "rg -u -g '!local.zsh' KEY .",
            "rg -u -g '!**/local.zsh' KEY",
            "rg -u --glob=!**/local.zsh KEY .",
            "rg --hidden --glob '!*.zsh' KEY .",
            "rg -u -g '!zsh/' KEY .",
            "ag -u -g '!local.zsh' KEY .",
            "ag -u --ignore local.zsh KEY .",
            "grep -rn --exclude=local.zsh KEY .",
            "grep -rn --exclude local.zsh KEY .",
            "grep -r --exclude='*.zsh' KEY .",
            "grep -rn --exclude-dir=zsh KEY .",
            "ack --ignore-dir=zsh KEY",
        ):
            self.assertIsNone(self.g15bash(cmd, cwd=repo), cmd)

    def test_g15_ps_on_macos(self):
        for cmd in (
            "ps -o pid=,ucomm= -p 1",
            "ps -c -p 1",
            "ps -ax -o pid,comm",
            "ps -o pid,etime,comm -p 123",
            "ps -p 1 -o etime=",
            "ps --help",
        ):
            self.assertIsNone(self.g15bash(cmd, platform="darwin"), cmd)
        for cmd in ("ps", "ps -p 1", "ps -U me", "ps -ux", "ps -j", "ps -v", "ps -O ppid -p 1", "ps -c -o args -p 1"):
            self.assertEqual(self.g15bash(cmd, platform="darwin"), self.G15_PS, cmd)
        for cmd in ("ps -p 1", "ps -U me", "ps -O ppid -p 1"):
            self.assertIsNone(self.g15bash(cmd, platform="linux"), cmd)

    def test_g15_grep_tool(self):
        repo = self.secret_tree()
        zsh = os.path.join(repo, "home", ".config", "zsh")
        for path in (
            "~/.config/zsh", "~/.config/zsh/local.zsh", zsh, os.path.join(zsh, "local.zsh"),
            os.path.join(self.dir, ".config", "zsh"),
        ):
            self.assertEqual(self.g15("Grep", {"pattern": "KEY", "path": path}), self.G15_FILE, path)
        self.assertEqual(self.g15("Grep", {"pattern": "KEY", "path": "local.zsh"}, cwd=zsh), self.G15_FILE)
        for path in (repo, "~", self.dir, "~/.config", os.path.join(zsh, "aliases.zsh"), "", None, 5):
            self.assertIsNone(self.g15("Grep", {"pattern": "KEY", "path": path}, cwd=repo), path)
        self.assertIsNone(self.g15("Grep", {"pattern": "KEY"}, cwd=repo))
        self.assertIsNone(self.g15("Grep", {"pattern": "KEY", "path": "~/.config/zsh"}, env={"GUARD_OFF": "G15"}))

    def test_g15_grep_tool_is_in_the_hook_matcher(self):
        settings = os.path.join(os.path.dirname(GUARD), "..", "settings.json")
        with open(settings) as f:
            hooks = json.load(f)["hooks"]["PreToolUse"]
        matchers = [h["matcher"] for h in hooks if "guard pretool" in json.dumps(h)]
        self.assertTrue(matchers and all("|Read|Grep" in m for m in matchers), matchers)

    def test_g15_letter_case_on_macos(self):
        self.secret_tree()
        # The deny needs a case-insensitive file system (macOS default): on a case-sensitive
        # one (Linux) ~/.config/ZSH is another folder and the check has nothing to resolve.
        if os.path.exists(os.path.join(self.dir, ".CONFIG")):
            for path in ("~/.config/ZSH/LOCAL.zsh", "~/.config/zsh/Local.ZSH"):
                self.assertEqual(self.g15("Read", {"file_path": path}, platform="darwin"), self.G15_FILE, path)
                self.assertEqual(self.g15bash("cat " + path, platform="darwin"), self.G15_FILE, path)
            self.assertEqual(self.g15bash("cat ~/.config/zsh/LOCAL.z*", platform="darwin"), self.G15_FILE)
            self.assertEqual(self.g15("Grep", {"path": "~/.config/ZSH"}, platform="darwin"), self.G15_FILE)
        self.assertIsNone(self.g15("Read", {"file_path": "~/.config/ZSH/LOCAL.zsh"}, platform="linux"))
        self.assertIsNone(self.g15bash("cat ~/.config/ZSH/LOCAL.zsh", platform="linux"))
        self.assertIsNone(self.g15("Read", {"file_path": "~/.config/ZSH/ALIASES.zsh"}, platform="darwin"))

    def test_g15_ln_install_and_dd(self):
        self.secret_tree()
        for cmd in (
            "ln -s ~/.config/zsh/local.zsh /tmp/x",
            "ln ~/.config/zsh/local.zsh /tmp/x",
            "install ~/.config/zsh/local.zsh /tmp/x",
            "dd if=~/.config/zsh/local.zsh",
            "dd if=$HOME/.config/zsh/local.zsh of=/dev/null",
            "dd of=/tmp/x if=~/.config/zsh/local.zsh bs=1",
        ):
            self.assertEqual(self.g15bash(cmd), self.G15_FILE, cmd)
        for cmd in (
            "dd if=/dev/zero of=/tmp/x count=1",
            "dd of=~/.config/zsh/local.zsh if=/dev/zero count=1",
            "ln -s ~/.config/zsh/aliases.zsh /tmp/x",
        ):
            self.assertIsNone(self.g15bash(cmd), cmd)

    def test_g15_off_switch_and_fail_open(self):
        self.assertIsNone(self.g15bash("ps aux", env={"GUARD_OFF": "G15"}))
        self.assertIsNone(self.g15bash("cat ~/.config/zsh/local.zsh", env={"GUARD_OFF": "G14 G15"}))
        self.assertIsNone(self.g15("Read", {"file_path": "~/.config/zsh/local.zsh"}, env={"GUARD_OFF": "g15"}))
        self.assertEqual(self.g15bash("ps aux", env={"GUARD_OFF": "G14"}), self.G15_PS)
        for tool, ti in (
            ("Bash", None),
            ("Bash", "oops"),
            ("Bash", {}),
            ("Bash", {"command": 5}),
            ("Bash", {"command": "echo 'unbalanced ps aux"}),
            ("Read", {}),
            ("Read", {"file_path": 7}),
            ("Read", "x"),
            ("Write", {"file_path": "~/.config/zsh/local.zsh"}),
        ):
            self.assertIsNone(self.g15(tool, ti), (tool, ti))
        with mock.patch.object(guard.os.path, "realpath", side_effect=RuntimeError("x")):
            self.assertIsNone(self.g15("Read", {"file_path": "~/.config/zsh/local.zsh"}))

    def test_g15_through_pretool(self):
        self.assertDeny(self.pre("Read", {"file_path": "~/.config/zsh/local.zsh"}), "G15: never read local.zsh")
        self.assertDeny(self.pre("Bash", {"command": "ps aux"}), "G15:")
        self.assertAllow(self.pre("Read", {"file_path": "/etc/hosts"}))
        self.assertAllow(self.pre("Bash", {"command": "pgrep -f node"}))
        env = dict(os.environ, TMPDIR=self.dir, HOME=self.dir, GUARD_OFF="G15")
        p = subprocess.run(
            [sys.executable, GUARD, "pretool"], input=json.dumps({"tool_name": "Bash", "tool_input": {"command": "ps aux"}}),
            capture_output=True, text=True, env=env, timeout=30,
        )
        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, "", ""))

    # G13 silence note
    NOTE13 = {
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": "G13: 8 tool calls with no text. Write one short status line for the user "
            "now, then go on.",
        }
    }

    @staticmethod
    def tu13(i):
        return {
            "type": "assistant",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "u%d" % i, "name": "Bash", "input": {}}]},
        }

    @staticmethod
    def say13(text):
        return {"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}

    def calls(self, start, end):
        out = []
        for i in range(start, end + 1):
            out += [self.tu13(i), tool_result("u%d" % i)]
        return out

    def post(self, records, last=None, **extra):
        path = self.transcript(records)
        data = {"hook_event_name": "PostToolUse", "session_id": "p1", "tool_name": "Bash", "transcript_path": path}
        if last is not None:
            data["tool_use_id"] = "u%d" % last
        data.update(extra)
        return self.run_guard("posttool", data)

    def test_g13_fires_at_exactly_eight(self):
        for n, fires in ((7, False), (8, True), (9, False), (20, False)):
            res = self.post([human()] + self.calls(1, n), last=n)
            if fires:
                self.assertEqual((res[0], res[2]), (0, ""))
                self.assertEqual(json.loads(res[1]), self.NOTE13)
                self.assertEqual(res[1].count("\n"), 1)
            else:
                self.assertAllow(res)

    def test_g13_off_in_the_desktop_app(self):
        recs = [human()] + self.calls(1, 8)
        res = self.post(recs, last=8)  # no entrypoint env: the note fires
        self.assertEqual(json.loads(res[1]), self.NOTE13)
        for entry, fires in (("claude-desktop", False), ("cli", True), ("", True)):
            with mock.patch.dict(os.environ, {"CLAUDE_CODE_ENTRYPOINT": entry}):
                res = self.post(recs, last=8)
            if fires:
                self.assertEqual(json.loads(res[1]), self.NOTE13, entry)
            else:
                self.assertAllow(res)

    def test_g13_without_tool_use_id_counts_all(self):
        res = self.post([human()] + self.calls(1, 8))
        self.assertEqual(json.loads(res[1]), self.NOTE13)
        self.assertAllow(self.post([human()] + self.calls(1, 9)))

    def test_g13_parallel_calls_each_see_their_own_count(self):
        recs = [human()] + self.calls(1, 10)
        fired = [i for i in range(1, 11) if self.post(recs, last=i)[1]]
        self.assertEqual(fired, [8])
        # a text written after the current call does not reset its count
        recs = [human()] + self.calls(1, 8) + [self.say13("Now checking the results")]
        self.assertEqual(json.loads(self.post(recs, last=8)[1]), self.NOTE13)

    def test_g13_text_resets_and_new_streak_starts(self):
        recs = [human()] + self.calls(1, 5) + [self.say13("Working on the parser now")] + self.calls(6, 10)
        self.assertAllow(self.post(recs, last=10))  # 5 since the text
        recs = [human()] + self.calls(1, 5) + [self.say13("Working on the parser now")] + self.calls(6, 13)
        self.assertEqual(json.loads(self.post(recs, last=13)[1]), self.NOTE13)  # 8 since the text
        # text in the same assistant message as a tool call counts as text
        mixed = {"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "text", "text": "Reading the config files"},
            {"type": "tool_use", "id": "u6", "name": "Bash", "input": {}}]}}
        recs = [human()] + self.calls(1, 5) + [mixed, tool_result("u6")] + self.calls(7, 11)
        self.assertAllow(self.post(recs, last=11))  # u6 shares a message with the text: 6 calls since it

    def test_g13_empty_text_does_not_reset(self):
        for filler in (" ", "\n", "...", ""):
            recs = [human()] + self.calls(1, 4) + [self.say13(filler)] + self.calls(5, 8)
            self.assertEqual(json.loads(self.post(recs, last=8)[1]), self.NOTE13, repr(filler))
        thinking = {"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "Let me think about this a lot"}]}}
        recs = [human()] + self.calls(1, 4) + [thinking] + self.calls(5, 8)
        self.assertEqual(json.loads(self.post(recs, last=8)[1]), self.NOTE13)

    def test_g13_stops_at_last_human_prompt(self):
        recs = self.calls(1, 6) + [human("go on")] + self.calls(7, 10)
        self.assertAllow(self.post(recs, last=10))  # only 4 since the prompt
        recs = self.calls(1, 6) + [human("go on")] + self.calls(7, 14)
        self.assertEqual(json.loads(self.post(recs, last=14)[1]), self.NOTE13)

    def test_g13_main_thread_only_and_off_switch(self):
        recs = [human()] + self.calls(1, 8)
        self.assertAllow(self.post(recs, last=8, agent_id="a1"))
        sub = os.path.join(self.dir, "x", "subagents")
        os.makedirs(sub)
        path = os.path.join(sub, "agent-a1.jsonl")
        with open(path, "w") as f:
            f.write("\n".join(json.dumps(r) for r in recs) + "\n")
        data = {"tool_name": "Bash", "tool_use_id": "u8", "transcript_path": path}
        self.assertAllow(self.run_guard("posttool", data))
        env = dict(os.environ, TMPDIR=self.dir, HOME=self.dir, GUARD_OFF="G13")
        path = self.transcript(recs)
        p = subprocess.run(
            [sys.executable, GUARD, "posttool"],
            input=json.dumps({"tool_name": "Bash", "tool_use_id": "u8", "transcript_path": path}),
            capture_output=True, text=True, env=env, timeout=30,
        )
        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, "", ""))

    def test_g13_fail_open(self):
        self.assertAllow(self.run_guard("posttool", raw="not json {{{"))
        self.assertAllow(self.run_guard("posttool", raw=""))
        self.assertAllow(self.run_guard("posttool", raw="[1]"))
        self.assertAllow(self.run_guard("posttool", {"tool_name": "Bash"}))
        self.assertAllow(self.run_guard("posttool", {"tool_name": "Bash", "transcript_path": "/no/such"}))
        path = os.path.join(self.dir, "bad.jsonl")
        with open(path, "w") as f:
            f.write("garbage\n{not json\n" * 5)
        self.assertAllow(self.run_guard("posttool", {"tool_name": "Bash", "transcript_path": path}))
        self.assertAllow(self.post([human()] + self.calls(1, 8), last=8, transcript_path=5))
        self.assertIsNone(guard.posttool({"transcript_path": "/no/such", "tool_use_id": "u8"}))
        with mock.patch.object(guard, "reverse_lines", side_effect=RuntimeError("x")):
            self.assertIsNone(guard.posttool({"transcript_path": "/x", "tool_use_id": "u8"}))


if __name__ == "__main__":
    unittest.main(verbosity=1)
