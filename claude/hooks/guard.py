#!/usr/bin/env python3
"""Claude Code guard hook (PreToolUse + UserPromptSubmit). Python 3 stdlib only.

Usage (wired in settings.json):
  python3 ~/.claude/hooks/guard.py pretool   # matcher Agent|CronCreate|ScheduleWakeup|Skill|AskUserQuestion
  python3 ~/.claude/hooks/guard.py prompt    # UserPromptSubmit

Guards: G1 Agent needs model+effort, G2 no polling at big context,
G3 context ceiling (nudge at 300K, deny Agent at 500K), G4 one AskUserQuestion per turn.

Deny = message on stderr + exit 2. Allow = exit 0 silently.
Fail-open: any exception, bad input or unknown mode exits 0 with no output.
"""
import json
import os
import re
import sys

POLL_CONTEXT = 100_000  # G2 applies above this
NUDGE_CONTEXT = 300_000  # G3 prompt nudge at/above this
SPAWN_CONTEXT = 500_000  # G3 Agent deny above this
MIN_INTERVAL_MIN = 30
NUDGE_EVERY = 5
MAX_EFFORT_WORDS = ("plan", "synthesis", "architecture", "final review")
_MAX_WORD = re.compile(r"\b(?:plan|synthesis|architecture|final\b.{0,30}\breview)\b")

MSG_AGENT = (
    'guard: Agent call needs "model" and "effort" (sonnet + high for build/review/'
    "research; opus or fable for plan and final review)."
)
MSG_MAX = (
    'guard: effort "max" is only for plan / synthesis / architecture / final review; '
    'use "high", or say which of those this is in the description.'
)
MSG_POLL = (
    "guard: no polling loops at big context; use Monitor / SendMessage "
    "notify_when_idle, or one final report."
)
MSG_ASK = "guard: one AskUserQuestion per turn; put all questions in one call."


# --- transcript reading -----------------------------------------------------


def reverse_lines(path, chunk=1 << 16):
    """Yield non-empty lines (bytes) of a file from the end, reading in chunks."""
    with open(path, "rb") as f:
        f.seek(0, 2)
        pos = f.tell()
        buf = b""
        while pos > 0:
            n = min(chunk, pos)
            pos -= n
            f.seek(pos)
            data = f.read(n) + buf
            parts = data.split(b"\n")
            buf = parts[0]
            if len(parts) == 1:
                chunk *= 2  # one giant line: read bigger pieces
            for line in reversed(parts[1:]):
                if line.strip():
                    yield line
        if buf.strip():
            yield buf


def _loads(line):
    try:
        rec = json.loads(line)
    except Exception:
        return None
    return rec if isinstance(rec, dict) else None


def _num(v):
    return v if isinstance(v, (int, float)) and not isinstance(v, bool) else 0


def context_size(path):
    """input + cache_creation + cache_read tokens of the last assistant usage record."""
    if not path:
        return 0
    try:
        for line in reverse_lines(path):
            if b"compact_boundary" in line or b"isCompactSummary" in line:
                rec = _loads(line)
                if rec and (rec.get("subtype") == "compact_boundary" or rec.get("isCompactSummary")):
                    return 0  # compacted since the last usage record: old size is stale
            if b'"assistant"' not in line:
                continue
            rec = _loads(line)
            if not rec or rec.get("type") != "assistant":
                continue
            msg = rec.get("message")
            usage = msg.get("usage") if isinstance(msg, dict) else None
            if not isinstance(usage, dict):
                continue
            total = (
                _num(usage.get("input_tokens"))
                + _num(usage.get("cache_creation_input_tokens"))
                + _num(usage.get("cache_read_input_tokens"))
            )
            if total > 0:  # skip all-zero synthetic records
                return int(total)
    except Exception:
        pass
    return 0


def is_human_prompt(rec):
    if rec.get("type") != "user" or rec.get("isMeta") or rec.get("isCompactSummary") or rec.get("isSidechain"):
        return False
    origin = rec.get("origin")
    if isinstance(origin, dict) and origin.get("kind") not in (None, "human"):
        return False
    msg = rec.get("message")
    content = msg.get("content") if isinstance(msg, dict) else None
    if isinstance(content, str):
        return True
    if isinstance(content, list):
        return not any(isinstance(b, dict) and b.get("type") == "tool_result" for b in content)
    return False


def asks_since_last_prompt(path, tool_use_id, tool_input):
    """Count AskUserQuestion tool_use blocks after the last human prompt, minus the current call."""
    found = []  # (id, input), newest first
    denied = set()  # tool_use ids whose result was an error (e.g. this guard's deny)
    for line in reverse_lines(path):
        if b'"user"' not in line and b"AskUserQuestion" not in line and b"askme" not in line:
            continue
        rec = _loads(line)
        if not rec:
            continue
        if is_human_prompt(rec):
            if b"ps:askme" in line:
                return 0  # a /ps:askme interview asks round after round in one turn
            break
        if rec.get("type") == "user":
            c = (rec.get("message") or {}).get("content") if isinstance(rec.get("message"), dict) else None
            for b in c if isinstance(c, list) else []:
                if isinstance(b, dict) and b.get("type") == "tool_result" and b.get("is_error"):
                    denied.add(b.get("tool_use_id"))
            continue
        if rec.get("type") != "assistant":
            continue
        msg = rec.get("message")
        content = msg.get("content") if isinstance(msg, dict) else None
        if not isinstance(content, list):
            continue
        for b in reversed(content):
            if isinstance(b, dict) and b.get("name") == "Skill" and "askme" in str((b.get("input") or {}).get("skill")):
                return 0
            if isinstance(b, dict) and b.get("type") == "tool_use" and b.get("name") == "AskUserQuestion":
                if b.get("id") not in denied:
                    found.append((b.get("id"), b.get("input")))
    # Only asks made BEFORE the current call count; a parallel sibling written
    # after it in the same message must not deny the first one.
    for idx, (i, inp) in enumerate(found):
        if (i == tool_use_id) if tool_use_id else (inp == tool_input):
            return len(found) - idx - 1
    return len(found)


# --- interval parsing (G2) ----------------------------------------------------


def _minute_set(field):
    """Parse a cron minute field made of numbers / ranges / lists; None if not parseable."""
    mins = set()
    for part in field.split(","):
        m = re.fullmatch(r"(\d+)(?:-(\d+))?(?:/(\d+))?", part)
        if not m:
            return None
        lo = int(m.group(1))
        hi = int(m.group(2)) if m.group(2) else lo
        step = int(m.group(3)) if m.group(3) else 1
        if step < 1 or lo > hi or hi > 59:
            return None
        mins.update(range(lo, hi + 1, step))
    return mins


def cron_interval_minutes(cron):
    """Smallest interval in minutes of a 5-field cron, or None if unknown."""
    fields = str(cron).split()
    if len(fields) != 5:
        return None
    minute, hour = fields[0], fields[1]
    if minute == "*":
        return 1
    m = re.fullmatch(r"\*/(\d+)", minute)
    if m:
        n = int(m.group(1))
        if n < 1:
            return None
        minute = "0-59/%d" % n  # */45 fires at :00 and :45, so the gap is 15
    if hour == "*" or not re.fullmatch(r"\d+", minute):
        mins = _minute_set(minute)
        if not mins:
            return None
        if len(mins) == 1:
            return 60
        ordered = sorted(mins)
        gaps = [b - a for a, b in zip(ordered, ordered[1:])]
        gaps.append(ordered[0] + 60 - ordered[-1])
        return min(gaps)
    h = re.fullmatch(r"\*/(\d+)", hour)
    if h and re.fullmatch(r"\d+", minute):
        n = int(h.group(1))
        return n * 60 if n >= 1 else None
    return None


_LOOP_TOKEN = re.compile(r"(?:\d+(?:h|min|m|s))+(?=\s|$)")  # scan-allowlist
_LOOP_PART = re.compile(r"(\d+)(h|min|m|s)")


_LOOP_EVERY = re.compile(r"(?<=\bevery )(?:\d+(?:h|min|m|s))+(?=\s|$)")  # scan-allowlist


def loop_interval_seconds(args):
    """Seconds of a leading interval token (30s, 5m, 10min, 1h, 2h30m); None if absent."""
    s = str(args or "").strip()
    m = _LOOP_TOKEN.match(s) or _LOOP_EVERY.search(s)
    if not m:
        return None
    mult = {"h": 3600, "min": 60, "m": 60, "s": 1}
    return sum(int(n) * mult[u] for n, u in _LOOP_PART.findall(m.group(0)))


# --- guards -------------------------------------------------------------------


class Ctx:
    """Lazy context-size lookup for the hook input."""

    def __init__(self, data):
        self.path = data.get("transcript_path")
        self._size = None

    @property
    def size(self):
        if self._size is None:
            self._size = context_size(self.path)
        return self._size


def g3_pretool(data, ctx):
    if data.get("tool_name") == "Agent" and ctx.size > SPAWN_CONTEXT:
        return (
            f"guard: context is {ctx.size // 1000}K (> 500K); run /ps:handoff and "
            "continue in a fresh session instead of spawning."
        )


def g1_agent(data, ctx):
    if data.get("tool_name") != "Agent":
        return
    ti = data.get("tool_input") or {}
    if not ti.get("effort") or not ti.get("model"):
        return MSG_AGENT
    if ti.get("effort") == "max":
        text = f"{ti.get('description') or ''} {ti.get('prompt') or ''}".lower()
        if not _MAX_WORD.search(text):
            return MSG_MAX


def g2_polling(data, ctx):
    tool = data.get("tool_name")
    if tool not in ("CronCreate", "ScheduleWakeup", "Skill"):
        return
    ti = data.get("tool_input") or {}
    if tool == "Skill" and ti.get("skill") != "loop":
        return
    if tool == "CronCreate" and ti.get("recurring") is False:
        return
    if ctx.size <= POLL_CONTEXT:
        return
    if tool == "CronCreate":
        iv = cron_interval_minutes(ti.get("cron"))
        bad = iv is not None and iv < MIN_INTERVAL_MIN
    elif tool == "ScheduleWakeup":
        delay = ti.get("delaySeconds")
        bad = isinstance(delay, (int, float)) and delay < MIN_INTERVAL_MIN * 60
    else:
        secs = loop_interval_seconds(ti.get("args"))
        bad = secs is not None and secs < MIN_INTERVAL_MIN * 60
    if bad:
        return MSG_POLL


def g4_ask(data, ctx):
    if data.get("tool_name") != "AskUserQuestion" or not ctx.path:
        return
    n = asks_since_last_prompt(ctx.path, data.get("tool_use_id"), data.get("tool_input"))
    if n >= 1:
        return MSG_ASK


def pretool(data):
    ctx = Ctx(data)
    for guard in (g3_pretool, g1_agent, g2_polling, g4_ask):
        msg = guard(data, ctx)
        if msg:
            return msg


def prompt(data):
    """G3 nudge: print JSON for stdout (or None); counter state lives in TMPDIR."""
    ctx = Ctx(data)
    sid = re.sub(r"[^A-Za-z0-9_.-]", "_", str(data.get("session_id") or "unknown"))
    state_dir = os.path.join(os.environ.get("TMPDIR") or "/tmp", "claude-guard")
    state_file = os.path.join(state_dir, sid + ".json")
    count = 0
    try:
        with open(state_file) as f:
            count = int(json.load(f).get("count", 0))
    except Exception:
        pass
    if ctx.size >= NUDGE_CONTEXT:
        count += 1
    else:
        count = 0
    try:
        os.makedirs(state_dir, exist_ok=True)
        with open(state_file, "w") as f:
            json.dump({"count": count}, f)
    except Exception:
        pass
    if count >= 1 and (count - 1) % NUDGE_EVERY == 0:
        return json.dumps(
            {
                "hookSpecificOutput": {
                    "hookEventName": "UserPromptSubmit",
                    "additionalContext": (
                        f"Context is {ctx.size // 1000}K. Run /ps:handoff and "
                        "continue in a fresh session."
                    ),
                }
            }
        )


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode not in ("pretool", "prompt"):
        return 0
    data = json.loads(sys.stdin.read())
    if not isinstance(data, dict):
        return 0
    if mode == "pretool":
        msg = pretool(data)
        if msg:
            sys.stderr.write(msg + "\n")
            return 2
    else:
        out = prompt(data)
        if out:
            sys.stdout.write(out + "\n")
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except Exception:
        code = 0
    sys.exit(code)
