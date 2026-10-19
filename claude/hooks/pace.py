#!/usr/bin/env python3
"""Weekly usage pace for the Pro plan. Python 3 stdlib only (3.9+).

Library (the guard imports it):
  pace.status(now=None)            -> {"state", "used", "budget", "source", "resets_at"}, fast, no scan
  pace.refresh(budget_s=5)         -> incremental scan of the transcripts; call from UserPromptSubmit
CLI:
  python3 pace.py status           print status() as JSON (refreshes first when the cache is old)
  python3 pace.py refresh          full refresh
  python3 pace.py seed <percent>   calibrate from the weekly percent the app reports

Sources: the statusline cache (claude-pace.json, written by statusline.sh) when fresh, else an
estimate from ~/.claude/projects transcripts. State: critical at 85% used, ahead when used is
above the budget for today (work week Monday to Saturday), else ok; unknown on no data or error.
Calibration (the weekly cost that means 100%) is automatic: refresh() reads the app's "You've hit
your weekly limit" record and stores the cost up to it; the statusline and `seed` also calibrate.
The newest calibration wins; the default is 5860 cost units.
Caches live in ${XDG_CACHE_HOME:-~/.cache}.
"""
import calendar
import datetime
import json
import math
import os
import sys
import time

WEEK = 7 * 86400
FORMAT = 2  # cache format: 1 held token weights; older caches are rebuilt or ignored
SCAN_FORMAT = 3  # scan cache: 3 added the weekly-limit hits per file
MIN_HIT_COST = 1000  # a limit-hit calibration below this cost comes from a partial scan
# "Weight" is cost units (USD-like, at API list prices), not tokens. The week of 10-12 was
# blocked at 5860 (the week before at 6320).
DEFAULT_WEEK_WEIGHT = 5860.0
# (input, output) price per million tokens. The fable prices are a guess.
PRICES = {"opus": (5, 25), "sonnet": (3, 15), "haiku": (1, 5), "fable": (15, 75)}
FRESH_S = 6 * 3600  # statusline cache is fresh for 6 hours
RESULT_TTL_S = 120  # cached estimate is current for 2 minutes
CALIBRATE_MIN = 5.0  # needs this much reported use to calibrate
CRITICAL = 85.0
CHECK_EVERY = 200  # lines between clock checks


def _cache_dir():
    return os.environ.get("XDG_CACHE_HOME") or os.path.join(os.path.expanduser("~"), ".cache")


def _path(name):
    return os.path.join(_cache_dir(), name)


def _projects_dir():
    base = os.environ.get("CLAUDE_CONFIG_DIR") or os.path.join(os.path.expanduser("~"), ".claude")
    return os.path.join(base, "projects")


def _read_json(path):
    try:
        with open(path, "r") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def _write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = "%s.%d.tmp" % (path, os.getpid())
    try:
        with open(tmp, "w") as f:
            json.dump(data, f, separators=(",", ":"))
        os.replace(tmp, path)
    except OSError:
        try:
            os.remove(tmp)
        except OSError:
            pass


def _epoch(v):
    """Seconds since the epoch from a number, a numeric string or an ISO UTC string; else None."""
    if isinstance(v, bool) or v is None:
        return None
    if isinstance(v, (int, float)):
        if not math.isfinite(v):
            return None
        return int(v / 1000) if v > 1e11 else int(v)
    if isinstance(v, str):
        try:
            return _epoch(float(v))
        except ValueError:
            pass
        try:
            return calendar.timegm(time.strptime(v[:19], "%Y-%m-%dT%H:%M:%S"))
        except ValueError:
            return None
    return None


def _num(v):
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v):
        return 0
    return v


def _pct(v):
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v):
        return None
    return float(min(100.0, max(0.0, v)))


# --- week and budget -------------------------------------------------------------------------

def _c1_reset(now):
    """The known weekly reset in the future (rolled forward by whole weeks), or None."""
    c1 = _read_json(_path("claude-pace.json"))
    if not isinstance(c1, dict) or not isinstance(c1.get("seven_day"), dict):
        return None
    e = _epoch(c1["seven_day"].get("resets_at"))
    if e is None:
        return None
    if e <= now:
        e += ((now - e) // WEEK + 1) * WEEK
    return e


def _fallback_week_start(now):
    d = datetime.datetime.fromtimestamp(now, datetime.timezone.utc)
    start = calendar.timegm((d.year, d.month, d.day, 6, 0, 0)) - d.weekday() * 86400
    return start if start <= now else start - WEEK


def _week_start(now):
    e = _c1_reset(now)
    return e - WEEK if e is not None else _fallback_week_start(now)


def _budget(now, ws):
    def day(t):
        lt = time.localtime(t)
        return datetime.date(lt.tm_year, lt.tm_mon, lt.tm_mday).toordinal()

    idx = max(0, day(now) - day(ws))
    return min(idx + 1, 6) / 6.0 * 100.0


# --- calibration -----------------------------------------------------------------------------

def _week_weight():
    c = _read_json(_path("claude-pace-calib.json"))
    w = c.get("week_weight") if isinstance(c, dict) and c.get("v") == FORMAT else None
    if isinstance(w, bool) or not isinstance(w, (int, float)) or not math.isfinite(w) or w <= 0:
        return DEFAULT_WEEK_WEIGHT
    return float(w)


def _calib_updated():
    """When the stored calibration was made (epoch), or 0 when there is none."""
    c = _read_json(_path("claude-pace-calib.json"))
    if not isinstance(c, dict) or c.get("v") != FORMAT or not isinstance(c.get("updated"), (int, float)):
        return 0
    return c["updated"]


def seed_weight(week_weight, now=None, source="seed", week_start=None):
    """Store the calibration. `updated` orders them: the newest calibration wins."""
    now = int(now if now is not None else time.time())
    _write_json(
        _path("claude-pace-calib.json"),
        {"v": FORMAT, "week_weight": week_weight, "updated": now, "source": source, "week_start": week_start},
    )


# --- statusline cache (C1) -------------------------------------------------------------------

def _fresh_c1(now):
    """(seven_day used %, resets_at) when the statusline cache is fresh, else None."""
    c1 = _read_json(_path("claude-pace.json"))
    if not isinstance(c1, dict) or not isinstance(c1.get("seven_day"), dict):
        return None
    updated = _epoch(c1.get("updated"))
    resets = _epoch(c1["seven_day"].get("resets_at"))
    used = _pct(c1["seven_day"].get("used_percentage"))
    if updated is None or resets is None or used is None:
        return None
    if not -3600 <= now - updated <= FRESH_S or resets <= now:
        return None
    return used, resets


# --- transcript scan -------------------------------------------------------------------------

def _prices(model):
    """(input, output) price per million tokens for a Claude model, else None (free or foreign)."""
    m = model.lower()
    if ":free" in m:
        return None
    for family in ("opus", "sonnet", "haiku", "fable"):
        if family in m:
            return PRICES[family]
    return None


def _cost(u, price):
    """Cost units of one usage block: cache writes cost 1.25x (5m) or 2x (1h), reads 0.1x."""
    cc = _num(u.get("cache_creation_input_tokens"))
    detail = u.get("cache_creation")
    if isinstance(detail, dict) and ("ephemeral_5m_input_tokens" in detail or "ephemeral_1h_input_tokens" in detail):
        w5 = _num(detail.get("ephemeral_5m_input_tokens"))
        w1 = _num(detail.get("ephemeral_1h_input_tokens"))
    else:
        w5, w1 = 0, cc
    pin, pout = price
    cin = _num(u.get("input_tokens")) + 1.25 * w5 + 2 * w1 + 0.1 * _num(u.get("cache_read_input_tokens"))
    return (pin * cin + pout * _num(u.get("output_tokens"))) / 1e6


def _parse(line, pos, ws):
    """(message key, [timestamp, weight]) for a counted assistant record, else None."""
    if b'"usage"' not in line:
        return None
    try:
        d = json.loads(line)
    except ValueError:
        return None
    msg = d.get("message") if isinstance(d, dict) else None
    if not isinstance(msg, dict) or not isinstance(msg.get("usage"), dict):
        return None
    model = msg.get("model")
    price = _prices(model) if isinstance(model, str) else None
    if price is None:
        return None
    ts = _epoch(d.get("timestamp"))
    if ts is None or ts < ws:
        return None
    weight = _cost(msg["usage"], price)
    mid = msg.get("id")
    key = mid if isinstance(mid, str) and mid else "L%d" % pos
    return key, [ts, weight]


def _hit_ts(line):
    """Timestamp of the app's "You've hit your weekly limit" record, else None.

    The record is an API error message (synthetic assistant) or a system record. A real reply
    that only quotes the phrase does not count."""
    try:
        d = json.loads(line)
    except ValueError:
        return None
    if not isinstance(d, dict):
        return None
    msg = d.get("message")
    texts = []
    if d.get("type") == "assistant" and isinstance(msg, dict):
        if not (d.get("isApiErrorMessage") or msg.get("model") == "<synthetic>"):
            return None
        content = msg.get("content")
        texts = [content] if isinstance(content, str) else []
        if isinstance(content, list):
            texts = [b.get("text") for b in content if isinstance(b, dict)]
    elif d.get("type") == "system":
        texts = [d.get("content"), msg.get("content") if isinstance(msg, dict) else None]
    if not any(isinstance(t, str) and "hit your weekly limit" in t.lower() for t in texts):
        return None
    return _epoch(d.get("timestamp"))


def _limit_hit_calibration(files, wk):
    """(cost up to the first weekly-limit hit of week wk, hit time) or None. Needs a complete scan."""
    hits = [t for e in files.values() for t in e.get("hits", ()) if wk <= t < wk + WEEK]
    if not hits:
        return None
    t0 = min(hits)
    cost = sum(w for e in files.values() for ts, w in e["msgs"].values() if wk <= ts <= t0)
    return (cost, t0) if cost > MIN_HIT_COST else None


def _find_files(root, min_mtime):
    found = []
    total = 0
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d != "tool-results"]
        for name in filenames:
            if name.endswith(".jsonl"):
                p = os.path.join(dirpath, name)
                try:
                    st = os.stat(p)
                except OSError:
                    continue
                total += 1
                if st.st_mtime >= min_mtime:
                    found.append((st.st_mtime, st.st_size, p))
    found.sort(reverse=True)  # newest first, so a short budget covers the live sessions
    return found, total


def _scan_file(path, ent, ws, deadline):
    """Read the new complete lines of one file into ent. False when the time budget ended."""
    pos = ent["offset"]
    n = 0
    try:
        with open(path, "rb") as f:
            f.seek(pos)
            for line in f:
                if not line.endswith(b"\n"):
                    break  # partial last line: wait for the writer
                got = _parse(line, pos, ws)
                if len(line) < 8000 and b"hit your weekly limit" in line.lower():  # short records only
                    t = _hit_ts(line)
                    if t is not None and t >= ws:
                        ent.setdefault("hits", []).append(t)
                pos += len(line)
                if got:
                    ent["msgs"][got[0]] = got[1]  # last record of a message wins
                n += 1
                if n % CHECK_EVERY == 0 and time.monotonic() >= deadline:
                    ent["offset"] = pos
                    return False
    except OSError:
        pass
    ent["offset"] = pos
    return True


def _sum(files, ws):
    total = 0.0
    for ent in files.values():
        for ts, w in ent["msgs"].values():
            if ws <= ts < ws + WEEK:
                total += w
    return total


def _valid_files(scan):
    files = scan.get("files") if isinstance(scan, dict) and scan.get("v") == SCAN_FORMAT else None
    if not isinstance(files, dict):
        return {}
    ok = {}
    for p, e in files.items():
        if isinstance(e, dict) and isinstance(e.get("offset"), int) and isinstance(e.get("msgs"), dict):
            ok[p] = e
    return ok


def _load_result():
    r = _read_json(_path("claude-pace-result.json"))
    if not isinstance(r, dict) or r.get("v") != FORMAT or not isinstance(r.get("weight"), (int, float)):
        return None
    if not isinstance(r.get("at"), (int, float)) or not isinstance(r.get("ws"), int):
        return None
    return r


def refresh(budget_s=5, now=None):
    """Scan new transcript bytes for at most budget_s seconds (one chunk minimum per call)."""
    now = int(now if now is not None else time.time())
    deadline = time.monotonic() + budget_s
    ws = _week_start(now)
    scan = _read_json(_path("claude-pace-scan.json"))
    old = _valid_files(scan)
    meta = scan if old or (isinstance(scan, dict) and scan.get("v") == SCAN_FORMAT) else {}
    backfilled = bool(meta.get("backfilled"))
    # A cold calibration looks at the previous week too, once, to find its limit hit.
    back = _calib_updated() == 0 and not backfilled
    lo = ws - WEEK if back else ws
    if not isinstance(meta.get("lo"), int) or meta["lo"] > lo:
        old = {}  # the cache covers less than this scan needs: rebuild
    files = {}
    dirty = old != _valid_files(scan)
    complete = True
    started = False
    found, total = _find_files(_projects_dir(), lo - 86400)
    for mtime, size, p in found:
        ent = old.get(p)
        if ent is None or size < ent["offset"]:
            ent = {"size": 0, "offset": 0, "mtime": 0, "msgs": {}, "hits": []}
            dirty = True
        files[p] = ent
        if size == ent["offset"] and ent.get("size") == size:
            continue
        if started and time.monotonic() >= deadline:
            complete = False
            continue
        started = True
        if not _scan_file(p, ent, lo, deadline):
            complete = False
        ent["size"], ent["mtime"] = size, mtime
        dirty = True
    if len(files) != len(old):
        dirty = True
    if complete:
        if back:
            backfilled, dirty = True, True
        # Weeks to look at: the previous one only on the backfill pass, then this one (newer wins).
        for wk in ((ws - WEEK, ws) if back else (ws,)):
            hit = _limit_hit_calibration(files, wk)
            if hit and hit[1] > _calib_updated():
                seed_weight(hit[0], hit[1], source="limit-hit", week_start=wk)
    if dirty or meta.get("lo") != lo:
        for ent in files.values():
            ent["msgs"] = {k: v for k, v in ent["msgs"].items() if v[0] >= lo}
            ent["hits"] = [t for t in ent.get("hits", ()) if t >= lo]
        _write_json(
            _path("claude-pace-scan.json"),
            {"v": SCAN_FORMAT, "lo": lo, "backfilled": backfilled, "files": files},
        )
    weight = _sum(files, ws)
    _write_json(
        _path("claude-pace-result.json"),
        {"v": FORMAT, "at": now, "ws": ws, "weight": weight, "files": total, "complete": complete},
    )
    if complete and weight > 0:
        c1 = _fresh_c1(now)
        if c1 and c1[0] >= CALIBRATE_MIN:
            seed_weight(weight / (c1[0] / 100.0), now, source="statusline", week_start=ws)
    return {"complete": complete, "weight": weight, "files": total}


def seed(used, now=None):
    """Calibrate the weekly weight from a percent the app reports. True when stored."""
    if isinstance(used, bool) or not isinstance(used, (int, float)) or not 0 < used <= 100:
        return False
    now = int(now if now is not None else time.time())
    res = refresh(budget_s=60, now=now)
    weight = res["weight"]
    if not res.get("complete") or weight <= 0:  # a partial scan would store a too-small weight
        return False
    seed_weight(weight / (used / 100.0), now)
    return True


# --- status ----------------------------------------------------------------------------------

def _state(used, budget):
    if used is None:
        return "unknown"
    if used >= CRITICAL:
        return "critical"
    return "ahead" if used > budget else "ok"


def _out(state, used, budget, source, resets_at):
    return {
        "state": state,
        "used": None if used is None else round(used, 1),
        "budget": round(budget, 1),
        "source": source,
        "resets_at": resets_at,
    }


def status(now=None):
    """Weekly pace from the caches only. Never raises; any error gives state "unknown"."""
    now = int(now if now is not None else time.time())
    try:
        ws = _week_start(now)
        budget = _budget(now, ws)
        c1 = _fresh_c1(now)
        if c1:
            return _out(_state(c1[0], budget), c1[0], budget, "statusline", c1[1])
        r = _load_result()
        if r and r["ws"] == ws and r.get("files", 0) > 0:
            used = min(100.0, r["weight"] / _week_weight() * 100.0)
            resets = _c1_reset(now)
            return _out(_state(used, budget), used, budget, "estimate", resets)
        return _out("unknown", None, budget, None, None)
    except Exception:
        try:
            budget = _budget(now, _fallback_week_start(now))
        except Exception:
            budget = 0.0
        return _out("unknown", None, budget, None, None)


def _needs_refresh(now):
    if _fresh_c1(now):
        return False
    r = _load_result()
    return r is None or abs(now - r["at"]) >= RESULT_TTL_S or r["ws"] != _week_start(now)


def main(argv):
    cmd = argv[1] if len(argv) > 1 else ""
    if cmd == "status" and len(argv) == 2:
        now = int(time.time())
        if _needs_refresh(now):
            refresh(budget_s=5, now=now)
        print(json.dumps(status(now)))
        return 0
    if cmd == "refresh" and len(argv) == 2:
        print(json.dumps(refresh(budget_s=120)))
        return 0
    if cmd == "seed" and len(argv) == 3:
        try:
            used = float(argv[2])
        except ValueError:
            used = None
        if used is None or not seed(used):
            sys.stderr.write("pace: seed needs a percent above 0 up to 100, and some usage this week\n")
            return 2
        print(json.dumps({"week_weight": _week_weight()}))
        return 0
    sys.stderr.write("usage: pace.py status | refresh | seed <percent>\n")
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
