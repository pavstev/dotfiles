#!/usr/bin/env python3
"""Tests for guard_cursor.py. Run: python3 cursor/hooks/test_guard_cursor.py"""
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "guard_cursor.py")
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)
import guard_cursor  # noqa: E402

guard = guard_cursor.load_guard()


def assistant(total):
    return {
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 10, "cache_creation_input_tokens": total - 10, "output_tokens": 5},
        },
    }


class GuardCursorTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = self.tmp.name
        self.app_db = os.path.join(self.dir, "no-app.vscdb")  # never the real app store

    def app_store(self, rows):
        """A fake Cursor.app state.vscdb: {conversation_id: composerData value}."""
        self.app_db = os.path.join(self.dir, "state.vscdb")
        con = sqlite3.connect(self.app_db)
        con.execute("CREATE TABLE IF NOT EXISTS cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)")
        for conv, value in rows.items():
            con.execute("INSERT INTO cursorDiskKV VALUES (?, ?)", ("composerData:" + conv, value))
        con.commit()
        con.close()

    def run_hook(self, mode, data=None, raw=None, script=SCRIPT):
        env = dict(os.environ, TMPDIR=self.dir, CURSOR_GUARD_APP_DB=self.app_db)
        stdin = raw if raw is not None else json.dumps(data)
        p = subprocess.run(
            [sys.executable, script, mode], input=stdin, capture_output=True, text=True, env=env, timeout=30
        )
        return p.returncode, p.stdout, p.stderr

    def transcript(self, total, name="t.jsonl"):
        path = os.path.join(self.dir, name)
        with open(path, "w") as f:
            f.write(json.dumps({"type": "user", "message": {"role": "user", "content": "hi"}}) + "\n")
            f.write(json.dumps(assistant(total)) + "\n")
        return path

    def seed(self, tokens, conv="c1"):
        """Record a context size the way Cursor would report one (afterAgentResponse)."""
        return self.run_hook(
            "response", {"hook_event_name": "afterAgentResponse", "conversation_id": conv,
                         "text": "ok", "context_tokens": tokens},
        )

    def compact(self, conv="c1"):
        return self.run_hook(
            "compact",
            {"hook_event_name": "preCompact", "conversation_id": conv, "context_tokens": 900_000,
             "context_window_size": 1_000_000, "context_usage_percent": 90},
        )

    def spawn(self, conv="c1", parent=None, **extra):
        data = {"hook_event_name": "subagentStart", "conversation_id": conv, "subagent_type": "general"}
        if parent:
            data["parent_conversation_id"] = parent
        data.update(extra)
        return self.run_hook("subagent", data)

    def prompt(self, conv="c1", **extra):
        data = {"hook_event_name": "beforeSubmitPrompt", "conversation_id": conv, "prompt": "go"}
        data.update(extra)
        rc, out, err = self.run_hook("prompt", data)
        self.assertEqual(rc, 0, (rc, out, err))
        out = json.loads(out)
        self.assertIs(out.get("continue"), True)
        return out

    def assertSilent(self, res):
        self.assertEqual(res, (0, "", ""))

    def assertDeny(self, res):
        rc, out, err = res
        self.assertEqual(rc, 2, res)
        body = json.loads(out)
        self.assertEqual(body["permission"], "deny")
        self.assertIn("/ps:handoff", body["user_message"])
        self.assertEqual(err.strip(), body["user_message"])

    # --- limits come from guard.py ---

    def test_reuses_guard_limits(self):
        self.assertIsNotNone(guard)
        self.assertEqual(guard.NUDGE_CONTEXT, 300_000)
        self.assertEqual(guard.SPAWN_CONTEXT, 350_000)

    # --- subagent ---

    def test_deny_above_spawn_context(self):
        self.seed(350_001)
        self.assertDeny(self.spawn())

    def test_allow_at_or_below_spawn_context(self):
        self.assertSilent(self.spawn())  # nothing known
        self.seed(350_000)
        self.assertSilent(self.spawn())

    def test_subagent_uses_parent_conversation(self):
        self.seed(600_000, conv="parent")
        self.assertDeny(self.spawn(conv="child", parent="parent"))
        self.assertSilent(self.spawn(conv="other"))

    def test_compact_clears_record(self):
        self.seed(600_000)
        self.assertDeny(self.spawn())
        self.assertSilent(self.compact())  # pre-compaction size is stale afterwards
        self.assertSilent(self.spawn())

    def test_transcript_usage_wins_over_record(self):
        self.seed(600_000)
        self.assertSilent(self.spawn(transcript_path=self.transcript(100_000)))
        self.assertDeny(self.spawn(transcript_path=self.transcript(550_000, "big.jsonl")))
        # unreadable or empty transcript falls back to the record
        self.assertDeny(self.spawn(transcript_path=os.path.join(self.dir, "missing.jsonl")))

    # --- Cursor.app chat store ---

    def test_app_store_size_denies_and_nudges(self):
        self.app_store({"c1": json.dumps({"contextTokensUsed": 600_000, "contextTokenLimit": 1_000_000})})
        self.assertDeny(self.spawn())
        rc, out, _ = self.run_hook("prompt", {"hook_event_name": "beforeSubmitPrompt", "conversation_id": "c1"})
        self.assertEqual(rc, 0)
        body = json.loads(out)
        self.assertIn("600K", body["user_message"])
        self.assertTrue(body["additional_context"].startswith("Start your reply by telling the user"))
        self.assertIn(body["user_message"], body["additional_context"])

    def test_app_store_small_size_wins_over_stale_record(self):
        self.seed(600_000)
        self.app_store({"c1": json.dumps({"contextTokensUsed": 32_539})})
        self.assertSilent(self.spawn())

    def test_app_store_breakdown_fallback_and_parent_conversation(self):
        self.app_store({"parent": json.dumps({"promptTokenBreakdown": {"totalUsedTokens": 700_000}})})
        self.assertDeny(self.spawn(conversation_id="child", parent_conversation_id="parent"))

    def test_app_store_bad_rows_and_missing_conversation_fall_back(self):
        self.app_store({"c1": "not json", "c2": json.dumps([1, 2]), "c3": json.dumps({"contextTokensUsed": "x"})})
        for conv in ("c1", "c2", "c3", "c4"):
            self.assertSilent(self.spawn(conversation_id=conv))
        self.seed(600_000, conv="c4")
        self.assertDeny(self.spawn(conversation_id="c4"))

    def test_app_store_not_a_database_fails_open(self):
        self.app_db = os.path.join(self.dir, "state.vscdb")
        with open(self.app_db, "w") as f:
            f.write("garbage")
        self.assertSilent(self.spawn())

    # --- response ---

    def test_response_usage_recorded(self):
        for i, payload in enumerate((
            {"usage": {"context_tokens": 600_000}},
            {"context_tokens": 600_000},
            {"tokens": {"contextTokens": 510_000}},
        )):
            with self.subTest(payload=payload):
                conv = f"r{i}"
                self.assertSilent(self.spawn(conv=conv))
                data = dict(payload, conversation_id=conv, text="done")
                self.assertSilent(self.run_hook("response", data))
                self.assertDeny(self.spawn(conv=conv))

    def test_pretooluse_task_gated_other_tools_ignored(self):
        self.seed(600_000)
        task = {"hook_event_name": "preToolUse", "conversation_id": "c1", "tool_name": "Task",
                "tool_input": {"prompt": "count"}}
        self.assertDeny(self.run_hook("subagent", task))
        self.assertSilent(self.run_hook("subagent", dict(task, tool_name="Shell")))

    def test_summed_input_tokens_ignored(self):
        # may be per-turn totals in Cursor; must not deny spawns
        data = {"conversation_id": "c1", "usage": {"input_tokens": 400_000, "cache_read_input_tokens": 200_000}}
        self.assertSilent(self.run_hook("response", data))
        self.assertSilent(self.spawn())

    def test_response_without_usage_is_noop(self):
        self.seed(600_000)
        self.assertSilent(self.run_hook("response", {"conversation_id": "c1", "text": "done"}))
        self.assertDeny(self.spawn())  # record untouched

    # --- prompt ---

    def test_prompt_below_threshold(self):
        self.assertNotIn("user_message", self.prompt())
        self.seed(299_000)
        self.assertNotIn("user_message", self.prompt())

    def test_nudge_cadence(self):
        self.seed(350_000)
        self.assertIn("/ps:handoff", self.prompt()["user_message"])  # 1st
        for _ in range(guard.NUDGE_EVERY - 1):
            self.assertNotIn("user_message", self.prompt())
        self.assertIn("user_message", self.prompt())  # 6th
        self.assertNotIn("user_message", self.prompt())

    def test_nudge_resets_below_threshold_and_conversations_independent(self):
        self.seed(350_000, conv="a")
        self.assertIn("user_message", self.prompt(conv="a"))
        self.assertNotIn("user_message", self.prompt(conv="b"))
        self.seed(100_000, conv="a")
        self.assertNotIn("user_message", self.prompt(conv="a"))
        self.seed(350_000, conv="a")
        self.assertIn("user_message", self.prompt(conv="a"))  # counter restarted

    # --- fail-open ---

    def test_fail_open(self):
        self.assertSilent(self.run_hook("prompt", raw="not json"))
        self.assertSilent(self.run_hook("subagent", raw="[1, 2]"))
        self.assertSilent(self.run_hook("bogus", {"conversation_id": "c1"}))
        self.assertSilent(self.run_hook("", raw=""))
        self.assertSilent(self.run_hook("compact", {}))  # missing fields
        self.assertSilent(self.run_hook("response", {}))
        self.assertSilent(self.spawn(conv=None))
        self.assertSilent(self.run_hook("compact", {"conversation_id": "c1", "context_tokens": "lots"}))

    def test_missing_guard_module_fails_open(self):
        lone = os.path.join(self.dir, "lone")
        os.makedirs(lone)
        copy = os.path.join(lone, "guard_cursor.py")
        with open(SCRIPT) as src, open(copy, "w") as dst:
            dst.write(src.read())
        self.assertSilent(self.run_hook("subagent", {"conversation_id": "c1"}, script=copy))

    # --- state ---

    def test_state_isolated_in_tmpdir(self):
        self.seed(600_000, conv="x/../y")
        files = os.listdir(os.path.join(self.dir, "cursor-guard"))
        self.assertEqual(files, ["x_.._y.json"])
        with open(os.path.join(self.dir, "cursor-guard", files[0])) as f:
            st = json.load(f)
        self.assertEqual(st["ctx"], 600_000)
        self.assertIn("updated", st)

    def test_state_pruned(self):
        d = os.path.join(self.dir, "cursor-guard")
        os.makedirs(d)
        old = time.time() - guard_cursor.MAX_STATE_AGE - 60
        for i in range(guard_cursor.MAX_STATE_FILES + 20):
            p = os.path.join(d, f"k{i}.json")
            with open(p, "w") as f:
                f.write("{}")
            if i < 10:
                os.utime(p, (old, old))
        self.seed(1000, conv="new")
        names = os.listdir(d)
        self.assertLessEqual(len(names), guard_cursor.MAX_STATE_FILES)
        self.assertIn("new.json", names)
        self.assertNotIn("k0.json", names)

    def test_conv_key(self):
        self.assertEqual(guard_cursor.conv_key({}, "prompt"), "unknown")
        self.assertEqual(guard_cursor.conv_key({"conversation_id": ".."}, "prompt"), "unknown")
        self.assertEqual(
            guard_cursor.conv_key({"conversation_id": "c", "parent_conversation_id": "p"}, "subagent"), "p"
        )
        self.assertEqual(
            guard_cursor.conv_key({"conversation_id": "c", "parent_conversation_id": "p"}, "prompt"), "c"
        )


if __name__ == "__main__":
    unittest.main()
