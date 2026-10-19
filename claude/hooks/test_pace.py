#!/usr/bin/env python3
"""Tests for pace.py. Run: python3 claude/hooks/test_pace.py"""
import calendar
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PACE = os.path.join(HERE, "pace.py")
sys.path.insert(0, HERE)
import pace  # noqa: E402

DAY = 86400


def utc(y, m, d, h=0, mi=0):
    return calendar.timegm((y, m, d, h, mi, 0))


def iso(epoch):
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(epoch)) + ".000Z"


# Monday 2026-10-19 06:00 UTC is a week start in the fallback rule.
MON = utc(2026, 10, 19, 6)


def record(epoch, mid, model="claude-sonnet-5-5", inp=1000, out=500, cc=2000, cr=10000):
    return {
        "type": "assistant",
        "timestamp": iso(epoch),
        "message": {
            "id": mid,
            "model": model,
            "role": "assistant",
            "usage": {
                "input_tokens": inp,
                "output_tokens": out,
                "cache_creation_input_tokens": cc,
                "cache_read_input_tokens": cr,
            },
        },
    }


# Default record on sonnet (3 / 15 per M): 3 x (1000 + 2 x 2000 + 0.1 x 10000) + 15 x 500, in millionths.
WEIGHT = (3 * (1000 + 2 * 2000 + 0.1 * 10000) + 15 * 500) / 1e6  # 0.0255


class PaceCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = self.tmp.name
        self.cache = os.path.join(self.home, "cache")
        os.makedirs(self.cache)
        self.saved = {k: os.environ.get(k) for k in ("HOME", "XDG_CACHE_HOME", "TZ", "CLAUDE_CONFIG_DIR")}
        os.environ["HOME"] = self.home
        os.environ["XDG_CACHE_HOME"] = self.cache
        os.environ.pop("CLAUDE_CONFIG_DIR", None)
        os.environ["TZ"] = "UTC"
        if hasattr(time, "tzset"):
            time.tzset()
        self.proj = os.path.join(self.home, ".claude", "projects", "proj")
        os.makedirs(self.proj)

    def tearDown(self):
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        if hasattr(time, "tzset"):
            time.tzset()
        self.tmp.cleanup()

    # helpers
    def write_lines(self, name, recs, sub=None, mode="w", mtime=MON + DAY):
        d = self.proj if sub is None else os.path.join(self.proj, sub, "subagents")
        os.makedirs(d, exist_ok=True)
        path = os.path.join(d, name)
        with open(path, mode) as f:
            for r in recs:
                f.write((r if isinstance(r, str) else json.dumps(r)) + "\n")
        os.utime(path, (mtime, mtime))  # the real clock must not matter
        return path

    def write_c1(self, updated, used, resets_at, five=None):
        data = {
            "updated": updated,
            "five_hour": {"used_percentage": five if five is not None else 1.0, "resets_at": resets_at - 5 * DAY},
            "seven_day": {"used_percentage": used, "resets_at": resets_at},
        }
        with open(os.path.join(self.cache, "claude-pace.json"), "w") as f:
            json.dump(data, f)

    def calib(self):
        with open(os.path.join(self.cache, "claude-pace-calib.json")) as f:
            return json.load(f)


class TestBudget(PaceCase):
    def budget_at(self, now):
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        pace.refresh(budget_s=5, now=now)
        return pace.status(now=now)

    def test_budget_per_weekday(self):
        want = [16.7, 33.3, 50.0, 66.7, 83.3, 100.0, 100.0]
        for i, w in enumerate(want):
            with self.subTest(day=i):
                st = self.budget_at(MON + i * DAY + 6 * 3600)
                self.assertAlmostEqual(st["budget"], w, places=1)

    def test_sunday_stays_at_100(self):
        st = self.budget_at(utc(2026, 10, 25, 23, 59))
        self.assertEqual(st["budget"], 100.0)

    def test_week_rollover(self):
        # Monday after 06:00 UTC starts a new week: old messages no longer count.
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        now = MON + 7 * DAY + 3600
        pace.refresh(budget_s=5, now=now)
        st = pace.status(now=now)
        self.assertAlmostEqual(st["budget"], 16.7, places=1)
        self.assertEqual(st["used"], 0.0)
        self.assertEqual(st["state"], "ok")

    def test_monday_before_six_belongs_to_previous_week(self):
        now = MON + 7 * DAY - 3 * 3600  # next Monday 03:00 UTC
        st = self.budget_at(now)
        self.assertEqual(st["budget"], 100.0)

    def test_week_start_from_resets_at(self):
        # Reset on Thursday 12:00: Thursday is day 0 of this week.
        thu = utc(2026, 10, 22, 12)
        now = thu + 2 * DAY + 3600  # Saturday
        self.write_c1(now - 60, 10.0, thu + 7 * DAY)
        st = pace.status(now=now)
        self.assertEqual(st["source"], "statusline")
        self.assertAlmostEqual(st["budget"], 50.0, places=1)  # day index 2 -> 3/6
        self.assertEqual(st["resets_at"], thu + 7 * DAY)


class TestSources(PaceCase):
    def test_unknown_without_data(self):
        st = pace.status(now=MON + 3600)
        self.assertEqual(st["state"], "unknown")
        self.assertIsNone(st["used"])
        self.assertIsNone(st["source"])

    def test_fresh_c1_wins(self):
        now = MON + 2 * DAY
        self.write_c1(now - 60, 41.2, MON + 7 * DAY)
        st = pace.status(now=now)
        self.assertEqual(st["source"], "statusline")
        self.assertEqual(st["used"], 41.2)
        self.assertEqual(st["state"], "ok")  # budget 50
        self.assertEqual(st["resets_at"], MON + 7 * DAY)

    def test_stale_c1_falls_back_to_estimate(self):
        now = MON + 2 * DAY
        self.write_c1(now - 7 * 3600, 41.2, MON + 7 * DAY)
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        pace.refresh(budget_s=5, now=now)
        st = pace.status(now=now)
        self.assertEqual(st["source"], "estimate")
        self.assertNotEqual(st["used"], 41.2)

    def test_c1_with_past_reset_is_not_fresh(self):
        now = MON + 8 * DAY
        self.write_c1(now - 60, 41.2, MON + 7 * DAY)
        st = pace.status(now=now)
        self.assertNotEqual(st["source"], "statusline")

    def test_corrupt_c1_is_ignored(self):
        with open(os.path.join(self.cache, "claude-pace.json"), "w") as f:
            f.write("{not json")
        self.assertEqual(pace.status(now=MON)["state"], "unknown")

    def test_c1_without_seven_day_is_ignored(self):
        with open(os.path.join(self.cache, "claude-pace.json"), "w") as f:
            json.dump({"updated": MON, "five_hour": {"used_percentage": 3, "resets_at": MON + 100}}, f)
        self.assertEqual(pace.status(now=MON + 10)["state"], "unknown")

    def test_states(self):
        now = MON + 2 * DAY  # Wednesday, budget 50
        for used, want in ((10, "ok"), (50, "ok"), (50.5, "ahead"), (84.9, "ahead"), (85, "critical"), (100, "critical")):
            with self.subTest(used=used):
                self.write_c1(now - 10, used, MON + 7 * DAY)
                self.assertEqual(pace.status(now=now)["state"], want)

    def test_critical_even_on_saturday(self):
        now = MON + 5 * DAY
        self.write_c1(now - 10, 90, MON + 7 * DAY)
        self.assertEqual(pace.status(now=now)["state"], "critical")

    def test_error_gives_unknown(self):
        orig = pace._read_json
        pace._read_json = lambda p: (_ for _ in ()).throw(RuntimeError("boom"))
        try:
            st = pace.status(now=MON)
        finally:
            pace._read_json = orig
        self.assertEqual(st["state"], "unknown")
        self.assertEqual(set(st), {"state", "used", "budget", "source", "resets_at"})


class TestEstimate(PaceCase):
    def estimate(self, now=None, week_weight=None):
        now = now or MON + DAY
        if week_weight:
            pace.seed_weight(week_weight)
        pace.refresh(budget_s=5, now=now)
        return pace.status(now=now)

    def test_weight_formula_and_percent(self):
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        st = self.estimate(week_weight=WEIGHT * 100)
        self.assertEqual(st["source"], "estimate")
        self.assertAlmostEqual(st["used"], 1.0, places=1)

    def test_default_week_weight(self):
        self.assertEqual(pace.DEFAULT_WEEK_WEIGHT, 5860.0)

    def cost_of(self, model, **kw):
        self.write_lines("a.jsonl", [record(MON + 60, "m1", model=model, **kw)])
        pace.refresh(budget_s=5, now=MON + DAY)
        weight = pace._load_result()["weight"]
        os.remove(os.path.join(self.proj, "a.jsonl"))  # transcripts only grow: start clean per call
        pace.refresh(budget_s=5, now=MON + DAY)
        return weight

    def test_cost_per_model_family(self):
        # input 1000, output 500, cache write 2000 (1h), cache read 10000.
        for model, (pi, po) in (
            ("claude-opus-4-8", (5, 25)),
            ("claude-sonnet-5-5", (3, 15)),
            ("claude-haiku-4-5", (1, 5)),
            ("fable-1", (15, 75)),
        ):
            with self.subTest(model=model):
                want = (pi * (1000 + 2 * 2000 + 0.1 * 10000) + po * 500) / 1e6
                self.assertAlmostEqual(self.cost_of(model), want)

    def test_cache_write_split_5m_and_1h(self):
        r = record(MON + 60, "m1", model="claude-opus-4-8", inp=0, out=0, cc=3000, cr=0)
        r["message"]["usage"]["cache_creation"] = {"ephemeral_5m_input_tokens": 1000, "ephemeral_1h_input_tokens": 2000}
        self.write_lines("a.jsonl", [r])
        pace.refresh(budget_s=5, now=MON + DAY)
        self.assertAlmostEqual(pace._load_result()["weight"], 5 * (1.25 * 1000 + 2 * 2000) / 1e6)

    def test_cache_write_without_split_counts_as_1h(self):
        self.assertAlmostEqual(
            self.cost_of("claude-opus-4-8", inp=0, out=0, cc=3000, cr=0), 5 * 2 * 3000 / 1e6
        )

    def test_old_cache_formats_are_rebuilt_or_ignored(self):
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        old = {"v": 1, "ws": MON, "files": {}}
        with open(os.path.join(self.cache, "claude-pace-scan.json"), "w") as f:
            json.dump(old, f)
        with open(os.path.join(self.cache, "claude-pace-result.json"), "w") as f:
            json.dump({"at": MON + DAY, "ws": MON, "weight": 1e9, "files": 5, "complete": True}, f)
        with open(os.path.join(self.cache, "claude-pace-calib.json"), "w") as f:
            json.dump({"week_weight": 1.3e9, "updated": MON}, f)
        self.assertEqual(pace.status(now=MON + DAY)["state"], "unknown")  # old result ignored
        self.assertEqual(pace._week_weight(), pace.DEFAULT_WEEK_WEIGHT)  # old calibration ignored
        pace.refresh(budget_s=5, now=MON + DAY)
        self.assertAlmostEqual(pace._load_result()["weight"], WEIGHT)
        scan = pace._read_json(os.path.join(self.cache, "claude-pace-scan.json"))
        self.assertEqual(scan["v"], pace.SCAN_FORMAT)

    def test_dedupe_streamed_records_last_wins(self):
        recs = [
            record(MON + 60, "m1", out=10),
            record(MON + 61, "m1", out=200),
            record(MON + 62, "m1", out=500),
        ]
        self.write_lines("a.jsonl", recs)
        self.estimate(week_weight=WEIGHT * 100)
        self.assertAlmostEqual(pace._load_result()["weight"], WEIGHT)

    def test_same_id_in_two_files_counts_per_file(self):
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        self.write_lines("b.jsonl", [record(MON + 60, "m1")])
        self.estimate()
        self.assertAlmostEqual(pace._load_result()["weight"], 2 * WEIGHT)

    def test_free_and_unknown_models_are_skipped(self):
        recs = [
            record(MON + 60, "m1", model="openai/gpt-oss-120b:free"),
            record(MON + 61, "m2", model="<synthetic>"),
            record(MON + 62, "m3", model="claude-opus-4-8"),
            record(MON + 63, "m4", model="claude-haiku-4-5"),
            record(MON + 64, "m5", model="fable-1"),
        ]
        self.write_lines("a.jsonl", recs)
        self.estimate()
        per = lambda pi, po: (pi * (1000 + 2 * 2000 + 0.1 * 10000) + po * 500) / 1e6  # noqa: E731
        self.assertAlmostEqual(pace._load_result()["weight"], per(5, 25) + per(1, 5) + per(15, 75))

    def test_old_messages_are_skipped(self):
        self.write_lines("a.jsonl", [record(MON - 60, "old"), record(MON + 60, "new")])
        self.estimate()
        self.assertAlmostEqual(pace._load_result()["weight"], WEIGHT)

    def test_corrupt_and_odd_lines_are_skipped(self):
        recs = [
            "{broken",
            "",
            '{"type":"assistant","message":"str"}',
            '{"type":"assistant","timestamp":"nope","message":{"id":"x","model":"claude-sonnet-5-5","usage":{"output_tokens":5}}}',
            '{"type":"assistant","timestamp":null,"message":{"usage":null}}',
            '{"usage": 1}',
            record(MON + 60, "m1"),
        ]
        self.write_lines("a.jsonl", recs)
        self.estimate()
        self.assertAlmostEqual(pace._load_result()["weight"], WEIGHT)

    def test_null_token_fields(self):
        r = record(MON + 60, "m1")
        r["message"]["usage"]["cache_read_input_tokens"] = None
        self.write_lines("a.jsonl", [r])
        self.estimate()
        self.assertAlmostEqual(pace._load_result()["weight"], (3 * (1000 + 2 * 2000) + 15 * 500) / 1e6)

    def test_subagent_files_are_counted(self):
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        self.write_lines("agent-1.jsonl", [record(MON + 70, "s1")], sub="sess1")
        self.estimate()
        self.assertAlmostEqual(pace._load_result()["weight"], 2 * WEIGHT)

    def test_incremental_scan_reads_only_new_bytes(self):
        path = self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        self.estimate()
        with open(path, "a") as f:
            f.write(json.dumps(record(MON + 90, "m2")) + "\n")
        pace.refresh(budget_s=5, now=MON + DAY)
        self.assertAlmostEqual(pace._load_result()["weight"], 2 * WEIGHT)
        scan = pace._read_json(os.path.join(self.cache, "claude-pace-scan.json"))
        self.assertEqual(scan["files"][path]["offset"], os.path.getsize(path))

    def test_partial_last_line_is_left_for_next_time(self):
        path = self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        with open(path, "a") as f:
            f.write(json.dumps(record(MON + 90, "m2"))[:40])  # no newline yet
        self.estimate()
        self.assertAlmostEqual(pace._load_result()["weight"], WEIGHT)
        with open(path, "a") as f:
            f.write(json.dumps(record(MON + 90, "m2"))[40:] + "\n")
        pace.refresh(budget_s=5, now=MON + DAY)
        self.assertAlmostEqual(pace._load_result()["weight"], 2 * WEIGHT)

    def test_truncated_file_is_rescanned(self):
        path = self.write_lines("a.jsonl", [record(MON + 60, "m1"), record(MON + 61, "m2")])
        self.estimate()
        self.write_lines("a.jsonl", [record(MON + 62, "m3")])
        self.assertLess(os.path.getsize(path), pace._read_json(os.path.join(self.cache, "claude-pace-scan.json"))["files"][path]["offset"])
        pace.refresh(budget_s=5, now=MON + DAY)
        self.assertAlmostEqual(pace._load_result()["weight"], WEIGHT)

    def test_status_uses_cached_result_under_120s(self):
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        now = MON + DAY
        self.estimate(now=now, week_weight=WEIGHT * 10)
        first = pace.status(now=now)["used"]
        self.write_lines("b.jsonl", [record(MON + 70, "m2")])
        self.assertEqual(pace.status(now=now + 30)["used"], first)  # no scan in status()
        pace.refresh(budget_s=5, now=now + 40)
        self.assertGreater(pace.status(now=now + 41)["used"], first)

    def test_status_ignores_result_from_another_week(self):
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        pace.refresh(budget_s=5, now=MON + DAY)
        self.assertEqual(pace.status(now=MON + 8 * DAY)["state"], "unknown")

    def test_no_transcripts_is_unknown(self):
        pace.refresh(budget_s=5, now=MON + DAY)
        self.assertEqual(pace.status(now=MON + DAY)["state"], "unknown")


class TestTimeBudget(PaceCase):
    def test_budget_stops_and_next_call_continues(self):
        recs = [record(MON + i, "m%d" % i) for i in range(1500)]
        self.write_lines("a.jsonl", recs)
        r = pace.refresh(budget_s=0, now=MON + DAY)
        self.assertFalse(r["complete"])
        self.assertLess(pace._load_result()["weight"], 1500 * WEIGHT)
        for _ in range(20):
            r = pace.refresh(budget_s=0, now=MON + DAY)
            if r["complete"]:
                break
        self.assertTrue(r["complete"])
        self.assertAlmostEqual(pace._load_result()["weight"], 1500 * WEIGHT)

    def test_budget_makes_progress_every_call(self):
        self.write_lines("a.jsonl", [record(MON + i, "m%d" % i) for i in range(1500)])
        pace.refresh(budget_s=0, now=MON + DAY)
        w1 = pace._load_result()["weight"]
        pace.refresh(budget_s=0, now=MON + DAY)
        self.assertGreater(pace._load_result()["weight"], w1)


class TestCalibration(PaceCase):
    def test_fresh_c1_stores_week_weight(self):
        now = MON + DAY
        self.write_lines("a.jsonl", [record(MON + 60, "m%d" % i) for i in range(10)])  # 45000
        self.write_c1(now - 10, 10.0, MON + 7 * DAY)
        pace.refresh(budget_s=5, now=now)
        self.assertAlmostEqual(self.calib()["week_weight"], 10 * WEIGHT / 0.10)

    def test_low_usage_does_not_calibrate(self):
        now = MON + DAY
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        self.write_c1(now - 10, 4.9, MON + 7 * DAY)
        pace.refresh(budget_s=5, now=now)
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))

    def test_stale_c1_does_not_calibrate(self):
        now = MON + DAY
        self.write_lines("a.jsonl", [record(MON + 60, "m1")])
        self.write_c1(now - 7 * 3600, 20.0, MON + 7 * DAY)
        pace.refresh(budget_s=5, now=now)
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))

    def test_incomplete_scan_does_not_calibrate(self):
        now = MON + DAY
        self.write_lines("a.jsonl", [record(MON + i, "m%d" % i) for i in range(1500)])
        self.write_c1(now - 10, 20.0, MON + 7 * DAY)
        r = pace.refresh(budget_s=0, now=now)
        self.assertFalse(r["complete"])
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))

    def test_estimate_uses_calibration(self):
        now = MON + DAY
        self.write_lines("a.jsonl", [record(MON + 60, "m%d" % i) for i in range(10)])
        self.write_c1(now - 10, 10.0, MON + 7 * DAY)
        pace.refresh(budget_s=5, now=now)
        os.remove(os.path.join(self.cache, "claude-pace.json"))
        pace.refresh(budget_s=5, now=now)
        st = pace.status(now=now)
        self.assertEqual(st["source"], "estimate")
        self.assertAlmostEqual(st["used"], 10.0, places=1)

    def test_seed(self):
        now = MON + DAY
        self.write_lines("a.jsonl", [record(MON + 60, "m%d" % i) for i in range(10)])
        pace.seed(25.0, now=now)
        self.assertAlmostEqual(self.calib()["week_weight"], 10 * WEIGHT / 0.25)
        self.assertAlmostEqual(pace.status(now=now)["used"], 25.0, places=1)

    def test_seed_needs_a_complete_refresh(self):
        from unittest import mock

        with mock.patch.object(pace, "refresh", return_value={"complete": False, "weight": 5e8, "files": 3}):
            self.assertFalse(pace.seed(25.0, now=MON + DAY))
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))
        with mock.patch.object(pace, "refresh", return_value={"complete": True, "weight": 5e8, "files": 3}):
            self.assertTrue(pace.seed(25.0, now=MON + DAY))
        self.assertAlmostEqual(self.calib()["week_weight"], 5e8 / 0.25)

    def test_seed_rejects_bad_values(self):
        for bad in (0, -3, 101, float("nan")):
            with self.subTest(bad=bad):
                self.assertFalse(pace.seed(bad, now=MON + DAY))
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))

    def test_corrupt_calibration_uses_default(self):
        with open(os.path.join(self.cache, "claude-pace-calib.json"), "w") as f:
            f.write("zzz")
        self.assertEqual(pace._week_weight(), pace.DEFAULT_WEEK_WEIGHT)


def hit(epoch, text="You've hit your weekly limit · resets Oct 19 at 8am (Europe/Vienna)", **extra):
    """The app's limit message: a synthetic assistant record flagged as an API error."""
    r = {
        "type": "assistant",
        "timestamp": iso(epoch),
        "isApiErrorMessage": True,
        "message": {
            "id": "hit%d" % epoch,
            "model": "<synthetic>",
            "role": "assistant",
            "content": [{"type": "text", "text": text}],
            "usage": {"input_tokens": 0, "output_tokens": 0},
        },
    }
    r.update(extra)
    return r


def big(epoch, mid, tokens=400_000_000):
    """One opus record worth tokens x 5 / 1e6 cost units (2000 by default)."""
    return record(epoch, mid, model="claude-opus-4-8", inp=tokens, out=0, cc=0, cr=0)


class TestLimitHit(PaceCase):
    def refresh(self, now=MON + 3 * DAY, **kw):
        return pace.refresh(budget_s=kw.pop("budget_s", 5), now=now)

    def test_hit_sets_calibration(self):
        t0 = MON + 2 * DAY
        self.write_lines("a.jsonl", [big(MON + 3600, "b1"), hit(t0), big(t0 + 60, "after", 100_000_000)])
        self.refresh()
        c = self.calib()
        self.assertAlmostEqual(c["week_weight"], 2000.0)  # the record after the hit is not counted
        self.assertEqual((c["source"], c["week_start"], c["updated"]), ("limit-hit", MON, t0))
        self.assertEqual(pace._week_weight(), c["week_weight"])
        # 2500 used against a 2000 week: capped at 100 and critical
        st = pace.status(now=MON + 3 * DAY)
        self.assertEqual((st["used"], st["state"]), (100.0, "critical"))

    def test_hit_in_a_second_file_and_subagent_counts(self):
        t0 = MON + 2 * DAY
        self.write_lines("a.jsonl", [big(MON + 3600, "b1", 200_000_000)])
        self.write_lines("agent-1.jsonl", [big(MON + 7200, "b2", 200_000_000)], sub="s1")
        self.write_lines("b.jsonl", [hit(t0)])
        self.refresh()
        self.assertAlmostEqual(self.calib()["week_weight"], 2000.0)

    def test_no_hit_keeps_default(self):
        self.write_lines("a.jsonl", [big(MON + 3600, "b1")])
        self.refresh()
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))
        self.assertEqual(pace._week_weight(), pace.DEFAULT_WEEK_WEIGHT)

    def test_a_reply_that_quotes_the_phrase_is_not_a_hit(self):
        quote = record(MON + 2 * DAY, "q1", model="claude-opus-4-8")
        quote["message"]["content"] = [{"type": "text", "text": "You've hit your weekly limit, they said"}]
        self.write_lines("a.jsonl", [big(MON + 3600, "b1"), quote])
        self.refresh()
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))

    def test_phrase_is_case_insensitive_and_system_records_count(self):
        sysrec = {"type": "system", "timestamp": iso(MON + 2 * DAY), "content": "YOU'VE HIT YOUR WEEKLY LIMIT"}
        self.write_lines("a.jsonl", [big(MON + 3600, "b1"), sysrec])
        self.refresh()
        self.assertAlmostEqual(self.calib()["week_weight"], 2000.0)

    def test_first_hit_of_the_week_wins(self):
        self.write_lines("a.jsonl", [big(MON + 3600, "b1"), hit(MON + DAY), big(MON + DAY + 60, "b2"), hit(MON + 3 * DAY)])
        self.refresh()
        self.assertAlmostEqual(self.calib()["week_weight"], 2000.0)

    def test_cheap_hit_stores_nothing(self):
        self.write_lines("a.jsonl", [big(MON + 3600, "b1", 100_000_000), hit(MON + DAY)])  # cost 500
        self.refresh()
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))

    def test_partial_scan_stores_nothing_until_complete(self):
        filler = [record(MON + 60 + i, "f%d" % i) for i in range(1500)]
        self.write_lines("a.jsonl", [big(MON + 3600, "b1")] + filler + [hit(MON + DAY)])
        r = self.refresh(budget_s=0)
        self.assertFalse(r["complete"])
        self.assertFalse(os.path.exists(os.path.join(self.cache, "claude-pace-calib.json")))
        for _ in range(20):
            if self.refresh(budget_s=0)["complete"]:
                break
        self.assertGreater(self.calib()["week_weight"], 2000.0)

    def test_newer_seed_wins(self):
        self.write_lines("a.jsonl", [big(MON + 3600, "b1"), hit(MON + 2 * DAY)])
        pace.seed_weight(3000.0, now=MON + 2 * DAY + 600)  # after the hit
        self.refresh()
        self.assertEqual(self.calib()["week_weight"], 3000.0)
        self.assertEqual(self.calib()["source"], "seed")

    def test_newer_hit_wins_over_older_seed(self):
        self.write_lines("a.jsonl", [big(MON + 3600, "b1"), hit(MON + 2 * DAY)])
        pace.seed_weight(3000.0, now=MON + DAY)  # before the hit
        self.refresh()
        self.assertAlmostEqual(self.calib()["week_weight"], 2000.0)

    def test_hit_is_stored_once(self):
        self.write_lines("a.jsonl", [big(MON + 3600, "b1"), hit(MON + 2 * DAY)])
        self.refresh()
        path = os.path.join(self.cache, "claude-pace-calib.json")
        os.utime(path, (1, 1))
        self.refresh()
        self.assertEqual(os.path.getmtime(path), 1)  # not rewritten

    def test_no_hit_this_week_keeps_last_week_calibration(self):
        self.write_lines("a.jsonl", [big(MON + 3600, "b1"), hit(MON + 2 * DAY)])
        self.refresh()
        nxt = MON + 8 * DAY
        self.write_lines("b.jsonl", [big(MON + 7 * DAY + 3600, "b9", 100_000_000)], mtime=nxt)
        self.refresh(now=nxt)
        self.assertAlmostEqual(self.calib()["week_weight"], 2000.0)
        self.assertAlmostEqual(pace.status(now=nxt)["used"], 500 / 2000 * 100)

    def test_backfill_finds_the_previous_week_hit_once(self):
        prev = MON - WEEK_S
        self.write_lines("a.jsonl", [big(prev + 3600, "b1"), hit(prev + 5 * DAY)])
        self.write_lines("b.jsonl", [big(MON + 3600, "b2", 100_000_000)])
        self.refresh(now=MON + DAY)
        c = self.calib()
        self.assertAlmostEqual(c["week_weight"], 2000.0)
        self.assertEqual((c["source"], c["week_start"]), ("limit-hit", prev))
        # this week's cost still counts only this week
        self.assertAlmostEqual(pace._load_result()["weight"], 500.0)

    def test_backfill_runs_once_without_a_hit(self):
        prev = MON - WEEK_S
        self.write_lines("a.jsonl", [big(prev + 3600, "b1")])
        self.refresh(now=MON + DAY)
        scan = pace._read_json(os.path.join(self.cache, "claude-pace-scan.json"))
        self.assertEqual((scan["lo"], scan["backfilled"]), (prev, True))
        self.refresh(now=MON + DAY + 10)
        scan = pace._read_json(os.path.join(self.cache, "claude-pace-scan.json"))
        self.assertEqual((scan["lo"], scan["backfilled"]), (MON, True))  # previous week dropped again

    def test_no_backfill_with_a_warm_calibration(self):
        prev = MON - WEEK_S
        pace.seed_weight(3000.0, now=MON)
        self.write_lines("a.jsonl", [big(prev + 3600, "b1"), hit(prev + 5 * DAY)])
        self.refresh(now=MON + DAY)
        self.assertEqual(self.calib()["week_weight"], 3000.0)


WEEK_S = 7 * DAY


class TestCli(PaceCase):
    def run_cli(self, *args):
        env = dict(os.environ)
        env.pop("PYTHONPATH", None)
        return subprocess.run([sys.executable, PACE, *args], capture_output=True, text=True, env=env, timeout=60)

    def test_status_prints_json(self):
        r = self.run_cli("status")
        self.assertEqual(r.returncode, 0, r.stderr)
        d = json.loads(r.stdout)
        self.assertEqual(set(d), {"state", "used", "budget", "source", "resets_at"})

    def test_status_cli_estimates_from_transcripts(self):
        now = int(time.time())
        self.write_lines("a.jsonl", [record(now - 60, "m1")])
        r = self.run_cli("status")
        d = json.loads(r.stdout)
        self.assertEqual(d["source"], "estimate")
        self.assertIsNotNone(d["used"])

    def test_refresh_and_seed(self):
        now = int(time.time())
        self.write_lines("a.jsonl", [record(now - 60, "m1")])
        self.assertEqual(self.run_cli("refresh").returncode, 0)
        r = self.run_cli("seed", "12.5")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertAlmostEqual(self.calib()["week_weight"], WEIGHT / 0.125)
        self.assertEqual(self.run_cli("seed", "abc").returncode, 2)

    def test_bad_usage_exits_2(self):
        self.assertEqual(self.run_cli().returncode, 2)
        self.assertEqual(self.run_cli("nope").returncode, 2)


class TestEpoch(unittest.TestCase):
    def test_epoch_forms(self):
        self.assertEqual(pace._epoch(1760860800), 1760860800)
        self.assertEqual(pace._epoch(1760860800000), 1760860800)  # milliseconds
        self.assertEqual(pace._epoch("1760860800"), 1760860800)
        self.assertEqual(pace._epoch("2025-10-19T08:00:00Z"), 1760860800)
        self.assertIsNone(pace._epoch(None))
        self.assertIsNone(pace._epoch("junk"))
        self.assertIsNone(pace._epoch(True))


if __name__ == "__main__":
    unittest.main(verbosity=1)
