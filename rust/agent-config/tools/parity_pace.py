#!/usr/bin/env python3
"""Differential test: claude/hooks/pace.py against the Rust `pace` binary.

Each case builds synthetic data (statusline cache, calibration, transcripts, old caches) in a temp
HOME, runs the same commands with both implementations from the same start state, and compares
stdout, stderr, the exit code and the JSON cache files left behind (floats within 1e-9).

Run: cargo build --release --manifest-path rust/agent-config/Cargo.toml
     python3 rust/agent-config/tools/parity_pace.py [cases] [seed]
"""
import calendar
import collections
import json
import math
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
PY = os.path.join(ROOT, "claude", "hooks", "pace.py")
RS = os.path.join(ROOT, "rust", "agent-config", "target", "release", "pace")
DAY = 86400
WEEK = 7 * DAY
COVER = collections.Counter()
TIME_KEYS = {"at", "updated"}  # set from the wall clock: the two runs are a second apart
MODELS = [
    "claude-opus-4-8", "claude-sonnet-5-5", "claude-haiku-4-5", "fable-1", "claude-OPUS-x",
    "openai/gpt-oss-120b:free", "<synthetic>", "gemini-2", None, 12,
]
PERCENTS = ["25", "12.5", "100", "0", "-5", "abc", "101", "nan", "1e1", "50", " 30 ", "inf", "1_0", "99.99", ""]
TZS = ["UTC", "Europe/Vienna", "America/Los_Angeles", "Asia/Kolkata", "Pacific/Auckland"]


def iso(t, ms=True):
    s = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t))
    return s + (".%03dZ" % random.randint(0, 999) if ms else "Z")


def fallback_ws(now):
    g = time.gmtime(now)
    start = calendar.timegm((g.tm_year, g.tm_mon, g.tm_mday, 6, 0, 0)) - g.tm_wday * DAY
    return start if start <= now else start - WEEK


def stamp(rng, t):
    k = rng.random()
    if k < 0.55:
        return iso(t)
    if k < 0.7:
        return iso(t, ms=False)
    if k < 0.78:
        return t
    if k < 0.84:
        return t * 1000
    if k < 0.88:
        return str(t)
    if k < 0.91:
        return float(t) + 0.5
    if k < 0.94:
        return "nope"
    if k < 0.96:
        return None
    if k < 0.98:
        return "2026-13-45T25:61:61Z"
    return "2026-2-3T4:5:6Z"


def tok(rng, big=False):
    k = rng.random()
    if big:
        return 400_000_000
    if k < 0.65:
        return rng.randint(0, 200_000)
    if k < 0.75:
        return rng.random() * 5000
    if k < 0.82:
        return None
    if k < 0.87:
        return "12"
    if k < 0.9:
        return True
    return rng.randint(0, 30)


def usage(rng, big=False):
    u = {}
    for name in ("input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"):
        if rng.random() < 0.9:
            u[name] = tok(rng, big and name == "input_tokens")
    k = rng.random()
    if k < 0.3:
        u["cache_creation"] = {"ephemeral_5m_input_tokens": tok(rng), "ephemeral_1h_input_tokens": tok(rng)}
    elif k < 0.38:
        u["cache_creation"] = {"ephemeral_1h_input_tokens": tok(rng)}
    elif k < 0.42:
        u["cache_creation"] = {}
    elif k < 0.45:
        u["cache_creation"] = "x"
    return u


def record(rng, t, mid, model=None, big=False):
    model = model if model is not None else rng.choice(MODELS)
    if big:
        model = "claude-opus-4-8"
    msg = {"role": "assistant", "model": model, "usage": usage(rng, big)}
    if mid is not None:
        msg["id"] = mid
    r = {"type": "assistant", "timestamp": stamp(rng, t), "message": msg}
    if rng.random() < 0.05:
        r["message"] = "str"
    if rng.random() < 0.03:
        msg["usage"] = None
    return r


def hit_line(rng, t):
    text = rng.choice([
        "You've hit your weekly limit · resets Oct 19 at 8am (Europe/Vienna)",
        "YOU'VE HIT YOUR WEEKLY LIMIT",
        "x" * 300 + " hit your weekly limit",
        "You’ve hit your weekly limit é",
    ])
    k = rng.random()
    if k < 0.45:
        return {"type": "assistant", "timestamp": iso(t), "isApiErrorMessage": True,
                "message": {"id": "hit%d" % t, "model": "<synthetic>", "content": [{"type": "text", "text": text}],
                            "usage": {"input_tokens": 0, "output_tokens": 0}}}
    if k < 0.6:
        return {"type": "assistant", "timestamp": stamp(rng, t), "message": {"model": "<synthetic>", "content": text}}
    if k < 0.75:
        return {"type": "system", "timestamp": iso(t), "content": text}
    if k < 0.82:
        return {"type": "system", "timestamp": iso(t), "message": {"content": text}}
    if k < 0.92:  # a real reply that quotes the phrase: not a hit
        return {"type": "assistant", "timestamp": iso(t),
                "message": {"id": "q%d" % t, "model": "claude-opus-4-8", "content": [{"type": "text", "text": text}],
                            "usage": usage(rng)}}
    if k < 0.96:
        return {"type": "assistant", "isApiErrorMessage": 1, "timestamp": iso(t),
                "message": {"content": [{"type": "text", "text": text}, "junk", {"nope": 1}]}}
    return {"type": "user", "timestamp": iso(t), "message": {"content": text}}


def odd_line(rng):
    return rng.choice([
        "{broken", "", "   ", '{"type":"assistant","message":"str"}', '{"usage": 1}', "[1,2,3]", "null", '"usage"',
        '{"type":"assistant","timestamp":"nope","message":{"id":"x","model":"claude-sonnet-5-5","usage":{"output_tokens":5}}}',
        '{"type":"assistant","timestamp":null,"message":{"usage":null}}',
        '{"message":{"usage":{"input_tokens":1},"model":"claude-haiku-4-5"},"timestamp":%d}' % int(time.time()),
    ])


def write_file(path, lines, mtime, partial=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        for ln in lines:
            f.write((ln if isinstance(ln, str) else json.dumps(ln)) + "\n")
        if partial:
            f.write(partial)
    os.utime(path, (mtime, mtime))


def make_lines(rng, ws, now, ids):
    lines = []
    n = rng.choice([0, 1, 3, 8, 20, 40])
    for _ in range(n):
        t = rng.randint(ws - 9 * DAY, now + DAY)
        k = rng.random()
        if k < 0.07:
            lines.append(hit_line(rng, rng.randint(ws - 7 * DAY, now)))
        elif k < 0.12:
            lines.append(odd_line(rng))
        else:
            mid = rng.choice([None, "", "m%d" % rng.randint(0, 6), "u%d" % len(lines) + str(rng.random())])
            if mid and mid.startswith("m"):
                ids.append(mid)
            lines.append(record(rng, t, mid, big=rng.random() < 0.08))
    return lines


def build_transcripts(rng, proj, ws, now, round_no=1):
    """Write 0 to 4 transcripts (some in subagent or ignored folders). Returns the paths."""
    paths = []
    for i in range(rng.choice([0, 1, 1, 2, 3, 4])):
        folder = rng.choice([proj, proj, os.path.join(proj, "sess1", "subagents"), os.path.join(proj, "prö j")])
        path = os.path.join(folder, "f%d_%d.jsonl" % (round_no, i))
        lines = make_lines(rng, ws, now, [])
        partial = None
        if rng.random() < 0.25:
            full = json.dumps(record(rng, now - 5, "p%d" % i, "claude-sonnet-5-5"))
            partial = full[: rng.randint(5, len(full) - 1)]
        if rng.random() < 0.1:
            lines.append("x " * 3000 + "You've hit your weekly limit")  # long record: never a hit
        mtime = now - rng.choice([10, 600, 3600, DAY, 3 * DAY, 10 * DAY, 20 * DAY])
        write_file(path, lines, mtime, partial)
        paths.append(path)
    if rng.random() < 0.2:
        write_file(os.path.join(proj, "tool-results", "big.jsonl"), [record(rng, now - 60, "tr")], now)
    if rng.random() < 0.15:
        write_file(os.path.join(proj, "notes.txt"), [record(rng, now - 60, "tx")], now)
    return paths


def write_cache(cache, name, data):
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(cache, name), "w") as f:
        f.write(data if isinstance(data, str) else json.dumps(data))


def make_c1(rng, cache, now):
    """Returns the resets_at it chose (or None for the fallback week)."""
    k = rng.random()
    if k < 0.3:
        return None
    resets = now + rng.choice([60, 3600, DAY, 2 * DAY + 5, 4 * DAY, 6 * DAY + 3000, WEEK - 100, -3 * DAY, -10 * DAY])
    upd = now - rng.choice([5, 60, 3600, 5 * 3600, 7 * 3600, 3 * DAY, -1800, -7200])
    used = rng.choice([0.0, 3, 5.0, 10, 33.333, 41.25, 49.99, 50.0, 70.5, 84.99, 85, 99.9, 100, 140, -4, 12.345, "x", None, True])
    five = rng.random() * 40
    form = rng.random()
    reset_v = resets if form < 0.7 else (resets * 1000 if form < 0.8 else (str(resets) if form < 0.9 else iso(resets)))
    upd_v = upd if rng.random() < 0.8 else (iso(upd) if rng.random() < 0.5 else upd * 1000)
    data = {"updated": upd_v, "five_hour": {"used_percentage": five, "resets_at": resets - 5 * DAY},
            "seven_day": {"used_percentage": used, "resets_at": reset_v}}
    r = rng.random()
    if r < 0.05:
        data = "{not json"
    elif r < 0.1:
        del data["seven_day"]
    elif r < 0.13:
        data["seven_day"] = {"used_percentage": 20}
    elif r < 0.15:
        data["seven_day"] = "x"
    write_cache(cache, "claude-pace.json", data)
    return resets


def week_start(c1_resets, now):
    if c1_resets is None:
        return fallback_ws(now)
    e = c1_resets
    if e <= now:
        e += ((now - e) // WEEK + 1) * WEEK
    return e - WEEK


def make_calib(rng, cache, now, ws):
    k = rng.random()
    if k < 0.55:
        return
    if k < 0.8:
        w = rng.choice([100.0, 2500.5, 5860.0, 12345.678, 1, 1e7])
        write_cache(cache, "claude-pace-calib.json", {"v": 2, "week_weight": w, "updated": now - rng.choice([10, DAY, 9 * DAY]), "source": "seed", "week_start": None})
    elif k < 0.88:
        write_cache(cache, "claude-pace-calib.json", {"week_weight": 1.3e9, "updated": now - 5})  # old format
    elif k < 0.94:
        write_cache(cache, "claude-pace-calib.json", "zzz")
    else:
        write_cache(cache, "claude-pace-calib.json", {"v": 2, "week_weight": rng.choice([0, -5, "x", None, True, 3.5]), "updated": rng.choice([0, None, "x", 1e9, True])})


def run_impl(cmd_prefix, args, env):
    p = subprocess.run(cmd_prefix + args, capture_output=True, text=True, env=env, timeout=300)
    return p.stdout, p.returncode, p.stderr


def make_env(base, tz):
    env = {k: v for k, v in os.environ.items() if k not in ("PYTHONPATH", "XDG_CACHE_HOME", "CLAUDE_CONFIG_DIR", "HOME", "TZ")}
    env.update({"HOME": base, "XDG_CACHE_HOME": os.path.join(base, "cache"), "CLAUDE_CONFIG_DIR": os.path.join(base, "claude"), "TZ": tz})
    return env


def snapshot(cache):
    snap = {}
    if os.path.isdir(cache):
        for name in sorted(os.listdir(cache)):
            with open(os.path.join(cache, name), "r") as f:
                text = f.read()
            try:
                snap[name] = json.loads(text)
            except ValueError:
                snap[name] = text
    return snap


def same(a, b, key=None, path=""):
    """Equal parsed JSON; floats within 1e-9; key order too; time keys within 5 s."""
    if isinstance(a, dict) and isinstance(b, dict):
        if list(a) != list(b):
            return "%s: keys %s != %s" % (path, list(a)[:6], list(b)[:6])
        for k in a:
            r = same(a[k], b[k], k, path + "/" + str(k)[-40:])
            if r:
                return r
        return None
    if isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            return "%s: len %d != %d" % (path, len(a), len(b))
        for i, (x, y) in enumerate(zip(a, b)):
            r = same(x, y, None, "%s[%d]" % (path, i))
            if r:
                return r
        return None
    if isinstance(a, bool) or isinstance(b, bool) or a is None or b is None or isinstance(a, str) or isinstance(b, str):
        return None if (a == b and type(a) is type(b)) else "%s: %r != %r" % (path, a, b)
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        if key in TIME_KEYS and abs(a - b) <= 5:
            return None
        if isinstance(a, int) and isinstance(b, int):
            return None if a == b else "%s: %r != %r" % (path, a, b)
        return None if math.isclose(a, b, rel_tol=1e-12, abs_tol=1e-9) else "%s: %r != %r" % (path, a, b)
    return "%s: %r != %r" % (path, a, b)


def run_case(rng, idx, work):
    base = os.path.join(work, "c%d" % idx)
    proj = os.path.join(base, "claude", "projects", "proj")
    cache0 = os.path.join(base, "cache0")
    os.makedirs(proj)
    os.makedirs(cache0)
    now = int(time.time())
    c1_resets = make_c1(rng, cache0, now)
    ws = week_start(c1_resets, now)
    make_calib(rng, cache0, now, ws)
    build_transcripts(rng, proj, ws, now)
    tz = rng.choice(TZS)
    env = make_env(base, tz)
    # An earlier run (either implementation) leaves caches; then the transcripts change.
    warm = rng.choice(["none", "none", "py", "rs", "old", "corrupt"])
    if warm in ("py", "rs"):
        shutil.copytree(cache0, os.path.join(base, "cache"))
        run_impl([sys.executable, PY] if warm == "py" else [RS], ["refresh"], env)
        shutil.rmtree(cache0)
        shutil.copytree(os.path.join(base, "cache"), cache0)
        shutil.rmtree(os.path.join(base, "cache"))
        if rng.random() < 0.6:
            build_transcripts(rng, proj, ws, now, round_no=2)
        if rng.random() < 0.4:  # a transcript grows, or is cut short
            files = [os.path.join(d, f) for d, _, fs in os.walk(proj) for f in fs if f.endswith(".jsonl")]
            if files:
                tgt = rng.choice(files)
                if rng.random() < 0.6:
                    with open(tgt, "a") as f:
                        f.write(json.dumps(record(rng, now - 3, "g%d" % rng.randint(0, 99), "claude-sonnet-5-5")) + "\n")
                else:
                    write_file(tgt, [record(rng, now - 3, "t1", "claude-haiku-4-5")], now)
        res = os.path.join(cache0, "claude-pace-result.json")
        if os.path.exists(res) and rng.random() < 0.5:
            r = json.load(open(res))
            r["at"] -= rng.choice([30, 500, 4000])
            write_cache(cache0, "claude-pace-result.json", r)
    elif warm == "old":
        write_cache(cache0, "claude-pace-scan.json", {"v": 1, "ws": ws, "files": {}})
        write_cache(cache0, "claude-pace-result.json", {"at": now, "ws": ws, "weight": 1e9, "files": 5, "complete": True})
    elif warm == "corrupt":
        write_cache(cache0, "claude-pace-scan.json", "{{{")
        write_cache(cache0, "claude-pace-result.json", {"v": 2, "at": now, "ws": ws, "weight": "x", "files": 3, "complete": True})
    cmds = []
    for _ in range(rng.choice([1, 1, 2, 3])):
        k = rng.random()
        cmds.append(["status"] if k < 0.4 else (["refresh"] if k < 0.65 else ["seed", rng.choice(PERCENTS)]))
    if idx % 40 == 7:
        cmds = [rng.choice([[], ["nope"], ["status", "x"], ["refresh", "1"], ["seed"], ["seed", "1", "2"]])]
    results = {}
    for name, prefix in (("py", [sys.executable, PY]), ("rs", [RS])):
        cache = os.path.join(base, "cache")
        if os.path.exists(cache0):
            shutil.copytree(cache0, cache)
        outs = [run_impl(prefix, c, env) for c in cmds]
        results[name] = (outs, snapshot(cache))
        shutil.rmtree(cache, ignore_errors=True)
    errs = []
    for c, a, b in zip(cmds, results["py"][0], results["rs"][0]):
        if a != b:
            errs.append("cmd %s: py=%r rs=%r" % (c, a, b))
    for out, rc, _ in results["py"][0]:
        for word in ('"statusline"', '"estimate"', '"unknown"', '"ahead"', '"critical"', '"week_weight"'):
            COVER[word] += word in out
        COVER["exit2"] += rc == 2
    calib = results["py"][1].get("claude-pace-calib.json")
    if isinstance(calib, dict):
        COVER["calib:" + str(calib.get("source"))] += 1
    d = same(results["py"][1], results["rs"][1])
    if d:
        errs.append("cache: " + d)
    shutil.rmtree(base, ignore_errors=True)
    desc = "case %d (tz %s, c1 %s, warm %s, cmds %s)" % (idx, tz, "yes" if c1_resets else "no", warm, cmds)
    return desc, errs


def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 100
    seed = int(sys.argv[2]) if len(sys.argv) > 2 else 20261019
    if not os.path.exists(RS):
        print("missing %s: build it first" % RS)
        return 1
    ok = 0
    with tempfile.TemporaryDirectory() as work:
        for i in range(n):
            random.seed(seed + i)  # iso() ms digits
            rng = random.Random(seed * 1000 + i)
            desc, errs = run_case(rng, i, work)
            if errs:
                print("DIFF " + desc)
                for e in errs[:4]:
                    print("   " + e[:600])
            else:
                ok += 1
    print("coverage: " + ", ".join("%s=%d" % kv for kv in sorted(COVER.items())))
    print("%d/%d identical" % (ok, n))
    return 0 if ok == n else 1


if __name__ == "__main__":
    sys.exit(main())
