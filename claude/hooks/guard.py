#!/usr/bin/env python3
"""Claude Code guard hook (PreToolUse, UserPromptSubmit, SubagentStop). Python 3 stdlib only.

Usage (wired in settings.json):
  python3 ~/.claude/hooks/guard.py pretool       # matcher Agent|EnterPlanMode|CronCreate|ScheduleWakeup|Skill|AskUserQuestion|SubagentHandback|archive_session|browser tools|Bash|Read|Grep
  python3 ~/.claude/hooks/guard.py prompt        # UserPromptSubmit
  python3 ~/.claude/hooks/guard.py subagentstop  # SubagentStop
  python3 ~/.claude/hooks/guard.py posttoolfailure  # PostToolUseFailure, matcher Agent: frees the slot
  python3 ~/.claude/hooks/guard.py posttool      # PostToolUse, every tool: G13 silence note
  python3 ~/.claude/hooks/guard.py scratch       # agent frontmatter hook, PreToolUse Write|Edit|NotebookEdit
  python3 ~/.claude/hooks/guard.py stop          # Stop

Guards: G1 Agent needs a deadline + hand-back cap, G2 no polling at big context,
G3 context ceiling (at 300K a note to keep PROGRESS.md current and let auto-compact carry the run,
deny Agent at 350K), G4 one AskUserQuestion per turn,
G5 subagent hand-back over its cap, G6 a hand-off written this turn (and still on disk) must reach
the user (Stop: the reply holds the file link and the paste block; PreToolUse archive_session: not
in the same turn). G7 no general-purpose / claude / untyped Agent. G8 helpers at once per session: 3, 1 when
the week is ahead of pace, 0 when critical. G9 plan gate: brainstorming / writing-plans only in
plan mode (unless the week is ahead); EnterPlanMode denied when ahead. Mode scratch: writes only
under temp or scratchpad folders.
Stop checks (main thread, one reminder each per turn, then quiet): G10 work left uncommitted or
unpushed in the session's repo (rule 13; the user's "do not push" or "local only" wins), G11 a
turn of 10 min or more that ends with no clock time (rule 12), G12 task-list steps still open
and nothing asked (quiet while a helper of this session still runs). Tool checks: G13 one note
after 8 tool calls in a row with no text (PostToolUse; off in the desktop app, which has its own
nudge), G14 deny BSD `sed -i` on macOS (use Edit or python3), G15 deny reading
~/.config/zsh/local.zsh (Read, Grep and Bash tools) and printing full process command lines.
G15 guards against accidents, not a boundary: interpreters (`python3 -c`, `node -e`) and
`source; env` still read the file.
Off switch: GUARD_OFF="G10,G13" (env, comma or space list) turns those checks off.
Router (main-thread Agent calls): sets model and effort (see route_agent); pace comes from pace.py.
Warnings (allow, add a note): W3 many browser calls in a main window, W4 a spawn or token budget.
UserPromptSubmit also adds a pace note when the week first runs ahead or critical.

Deny = message on stderr + exit 2. Allow = exit 0 silently.
Warn = exit 0 with hookSpecificOutput.additionalContext JSON on stdout (the router adds
permissionDecision allow + updatedInput to the same line).
Fail-open: any exception, bad input or unknown mode exits 0 with no output.
"""
import calendar
import contextlib
import fnmatch
import json
import os
import re
import shlex
import subprocess
import sys
import time

try:
    import fcntl  # POSIX only: locks the per-session state file
except ImportError:  # Windows
    fcntl = None

try:
    import pace  # claude/hooks/pace.py (contract C2); None = treat the week as "unknown"
except Exception:
    pace = None

POLL_CONTEXT = 100_000  # G2 applies above this
NUDGE_CONTEXT = 300_000  # G3 prompt nudge at/above this
SPAWN_CONTEXT = 350_000  # G3 Agent deny above this (autocompact is 350K)
MIN_INTERVAL_MIN = 30
NUDGE_EVERY = 5
SPAWN_TTL = 90 * 60  # a spawn older than this no longer counts as running
CAPS = {"ok": 3, "unknown": 3, "ahead": 1, "critical": 0}  # helpers at once, by pace state
PLAN_SKILLS = ("superpowers:brainstorming", "superpowers:writing-plans")
LOOKUP_AGENTS = ("Explore", "claude-code-guide", "statusline-setup")

# G1 brief check: the prompt must name a deadline and a hand-back cap.
DEADLINE_RE = re.compile(
    r"deadline|time ?(?:box|limit)|stop at|\d+ ?-?(?:m|mins?|minutes?|h|hrs?|hours?)\b", re.I
)
CAP_RE = re.compile(
    r"hand[ -]?back|at most \d[\d,]*|\d[\d,]* ?(?:chars|characters|words|lines)\b|under \d[\d,]*"
    r"|max(?:imum)? \d[\d,]*|one line|≤ ?\d[\d,]*",
    re.I,
)

# G5 hand-back cap.
HANDBACK_SLACK = 1.10
HANDBACK_MAX_DENIES = 2
HANDBACK_DEFAULT_CAP = 4000
HANDBACK_CAPS = {
    "task-implementer": 3000,
    "task-reviewer": 3000,
    "code-reviewer": 4000,
    "researcher": 3000,
    "repo-scanner": 2000,
    "ui-verifier": 3000,
    "worker": 3000,
}
_CAP_NUM = re.compile(r"(\d[\d,]*) ?(?:chars|characters)\b", re.I)
_CAP_HANDBACK = re.compile(r"hand[ -]?backs?\b", re.I)
_CAP_STRONG = re.compile(r"final message|reply|response|respond|output", re.I)
_CAP_WEAK = re.compile(r"answer|return|report|summary", re.I)
_CAP_BOUND = re.compile(r"(?:≤|<=|at most|under|max(?:imum)?|stay|keep|limit|no more than|within)\W*$", re.I)
_SENTENCE_END = re.compile(r"\.\s|\n")
PROMPT_SCAN_LINES = 200
SCRATCH_ROOTS = ("/tmp", "/private/tmp", "/var/folders", "/private/var/folders")
SCRATCH_DIRS = {".agents", ".superpowers"}  # git-ignored note folders where callers ask for reports

MSG_BRIEF = (
    "guard: Agent prompt needs {missing}. Add `Deadline: 25 min.` and "
    "`Hand back at most 4,000 chars; write the rest to a file.`"
)
MSG_HANDBACK = (
    "guard: hand-back is {n} chars; the cap is {cap}. Write the full text to a file (the path "
    "the caller named, or the scratchpad), then hand back at most {cap} chars: status on line 1, "
    "key results, the file path. No Write tool: cut to the top findings."
)
MSG_SCRATCH = (
    "guard: this agent writes only scratch files; write under the scratchpad, $TMPDIR, "
    ".agents/ or .superpowers/."
)
MSG_POLL = (
    "guard: no polling loops at big context; use Monitor / SendMessage "
    "notify_when_idle, or one final report."
)
MSG_ASK = "guard: one AskUserQuestion per turn; put all questions in one call."
MSG_HANDOFF_REPLY = (
    "guard: you wrote the hand-off {path} this turn, but your reply does not give it to the user. "
    "End with the file link [{name}]({path}) and a fenced block that holds only: "
    "Read {path} and do it."
)
MSG_HANDOFF_ARCHIVE = (
    "guard: you wrote the hand-off {path} this turn. Do not archive yet: give the link "
    "[{name}]({path}) and the block `Read {path} and do it.`, end the turn, and archive "
    "after the user answers."
)
MSG_HEAVY = (
    "guard: a {kind} subagent loads every tool and skill (about 60K tokens; `worker` starts near "
    "15-20K). Use `worker` (edits, commands), `researcher` (web), `Explore` (search), "
    "`task-implementer` (one plan task) or `Plan` (planning)."
)
HEAVY_AGENTS = {"general-purpose", "claude"}  # no tools: list, so they load every tool and skill
MSG_CAP = (
    "guard: {n} {helpers} running, the cap is {cap} (week {used} used, budget {budget}, pace {state}). "
    "Wait for one to finish, or do the step yourself."
)
MSG_PLAN_GATE = (
    "guard: plans run on Opus only in plan mode (opusplan). Call EnterPlanMode first, then save the "
    "plan file after you approve it (autopilot: send a `Plan` helper instead)."
)
MSG_PLAN_GATE_APP = (
    "guard: in the desktop app plan mode does not switch a Sonnet session to Opus. For this plan, "
    "send a `Plan` helper (Opus by the router), or pick Opus in the model menu."
)
MSG_NO_PLAN_MODE = (
    "guard: the week is {state} (used {used}, budget {budget}). Plan without plan mode so Sonnet "
    "plans."
)
MSG_PACE_AHEAD = (
    "guard: the week is ahead of pace ({used} used, budget {budget} today). Helpers run on Sonnet, "
    "max 1 at once, and plans go without plan mode. If the main model is Opus, tell the user once "
    "to pick Sonnet."
)
MSG_PACE_CRITICAL = (
    "guard: the week is critical ({used} used, budget {budget} today). Sonnet only, no helpers. "
    "Tell the user once."
)
BROWSER_PREFIXES = ("mcp__Claude_Browser__", "mcp__claude-in-chrome__")
BROWSER_WARN_AT = 5  # earlier browser calls this turn before the first note
BROWSER_WARN_EVERY = 20
MSG_BROWSER = (
    "guard (warn): {n} browser calls in this window this turn. Page dumps and screenshots fill the "
    "main context. For a check (layout, console, a click-through flow, a live URL), dispatch "
    "`ui-verifier`; it hands back pass or fail with evidence. Keep going here only when the user "
    "asked you to drive the browser for them."
)
SPAWN_CAP = 15  # Agent spawns per session before the budget note
TOKEN_CAP = 1_500_000  # sum of the totalTokens the Agent results report
BUDGET_EVERY = 5
MSG_BUDGET = (
    "guard (warn): budget. This session has spawned {n} agents ({tok}K subagent tokens; caps {cap} "
    "agents, {tcap}K). Before more: call mcp__ccd_session_mgmt__get_usage, run the fan-out gate "
    "(parallel parts? pause? recovery? record? all no means no fan-out), and batch small tasks."
)
_PANEL_ANGLE = re.compile(r"\b(?:panel|angle|re-?review)\b", re.I)  # ps:review-panel sonnet angles
_FINAL_REVIEW = re.compile(r"\bfinal\b.{0,30}\breview|\bwhole[- ]branch\b|\bbranch[- ]review\b", re.I)
ARCHIVE_TOOL = "mcp__ccd_session_mgmt__archive_session"
_HANDOFF_PATH = re.compile(r"/handoffs/[^/]+\.md$")


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


def check_off(gid):
    """True when the user turned check `gid` off: GUARD_OFF="G10,G13" in the env."""
    return gid.upper() in re.split(r"[\s,]+", os.environ.get("GUARD_OFF", "").upper())


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


def pace_status():
    """pace.status() as a dict whose state is one of CAPS; any failure is "unknown"."""
    try:
        st = pace.status()
        if isinstance(st, dict) and st.get("state") in CAPS:
            return st
    except Exception:
        pass
    return {"state": "unknown", "used": None, "budget": None, "source": None, "resets_at": None}


def _pct(v):
    return "%.0f%%" % v if isinstance(v, (int, float)) and not isinstance(v, bool) else "?"


def _tool_input(data):
    ti = data.get("tool_input")
    return ti if isinstance(ti, dict) else {}


def is_main_thread(data):
    return not data.get("agent_id") and "/subagents/" not in str(data.get("transcript_path") or "")


class Ctx:
    """Lazy context-size and pace lookup for the hook input."""

    def __init__(self, data):
        self.path = data.get("transcript_path")
        self._size = None
        self._pace = None

    @property
    def size(self):
        if self._size is None:
            self._size = context_size(self.path)
        return self._size

    @property
    def pace(self):
        if self._pace is None:
            self._pace = pace_status()
        return self._pace


def g3_pretool(data, ctx):
    if data.get("tool_name") == "Agent" and ctx.size > SPAWN_CONTEXT:
        return (
            f"guard: context is {ctx.size // 1000}K (> {SPAWN_CONTEXT // 1000}K); do this step yourself "
            "instead of spawning a helper. Keep PROGRESS.md current; auto-compact carries the run."
        )


def g1_agent(data, ctx):
    """G1: the brief names a deadline and a hand-back cap (the router fills model and effort)."""
    if data.get("tool_name") != "Agent" or not isinstance(data.get("tool_input"), dict):
        return
    ti = data["tool_input"]
    prompt_text = ti.get("prompt")
    prompt_text = prompt_text if isinstance(prompt_text, str) else ""
    missing = []
    if not DEADLINE_RE.search(prompt_text):
        missing.append("a deadline")
    if not CAP_RE.search(prompt_text):
        missing.append("a hand-back cap")
    if missing:
        return MSG_BRIEF.format(missing=" and ".join(missing))


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


def _text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = [b.get("text") for b in content if isinstance(b, dict) and b.get("type") == "text"]
        parts = [p for p in parts if isinstance(p, str)]
        return "\n".join(parts) if parts else None
    return None


def subagent_prompt(path):
    """Text of the first user record in a subagent transcript (read from the start, 200 lines max)."""
    if not path:
        return None
    try:
        with open(path, "rb") as f:
            for i, line in enumerate(f):
                if i >= PROMPT_SCAN_LINES:
                    break
                if b'"user"' not in line:
                    continue
                rec = _loads(line)
                if not rec or rec.get("type") != "user":
                    continue
                msg = rec.get("message")
                text = _text_of(msg.get("content") if isinstance(msg, dict) else None)
                if text:
                    return text
    except Exception:
        pass
    return None


def subagent_transcript(data):
    atp = data.get("agent_transcript_path")
    if isinstance(atp, str) and atp:
        return atp
    tp = data.get("transcript_path")
    if not isinstance(tp, str) or not tp:
        return None
    if "/subagents/" in tp:
        return tp
    aid = data.get("agent_id")
    if not aid:
        return None
    base = tp[: -len(".jsonl")] if tp.endswith(".jsonl") else tp
    return "%s/subagents/agent-%s.jsonl" % (base, aid)


def prompt_cap(prompt_text):
    """Smallest N (>= 200) of 'N chars' that the prompt ties to the reply, or None.

    The tie word must sit in the same sentence: up to 80 chars before the number or 40 after.
    A "hand back" tie wins, then strong ties (final message, reply, output ...), then weak ones
    (return, summary ...), so spec text like "return at most 500 characters" or "a hard cap of 2,500
    chars on the final message" in a file the agent edits does not beat the caller's hand-back cap.
    Last comes a bare bound right before the number ("Stay ≤ 4,000 chars.").
    """
    handback, strong, weak, bound = [], [], [], []
    text = prompt_text or ""
    for m in _CAP_NUM.finditer(text):
        try:
            n = int(m.group(1).replace(",", ""))
        except ValueError:
            continue
        if n < 200:
            continue
        before = text[max(0, m.start() - 80) : m.start()]
        before = _SENTENCE_END.split(before)[-1]
        after = _SENTENCE_END.split(text[m.end() : m.end() + 40])[0]
        window = before + " " + after
        if _CAP_HANDBACK.search(window):
            handback.append(n)
        elif _CAP_STRONG.search(window):
            strong.append(n)
        elif _CAP_WEAK.search(window):
            weak.append(n)
        elif _CAP_BOUND.search(before[-25:]):
            bound.append(n)  # "Stay ≤ 4,000 chars." with no reply word
    pick = handback or strong or weak or bound
    return min(pick) if pick else None


def handback_cap(data):
    cap = prompt_cap(subagent_prompt(subagent_transcript(data)))
    if cap:
        return cap
    return HANDBACK_CAPS.get(data.get("agent_type"), HANDBACK_DEFAULT_CAP)


def _state_dir():
    return os.path.join(os.environ.get("TMPDIR") or "/tmp", "claude-guard")


def check_handback(data, text):
    """G5: deny a hand-back over cap x 1.10, at most twice per agent; None = allow."""
    if not isinstance(text, str) or not text:
        return
    cap = handback_cap(data)
    if len(text) <= cap * HANDBACK_SLACK:
        return
    key = re.sub(r"[^A-Za-z0-9_.-]", "_", str(data.get("agent_id") or data.get("session_id") or "unknown"))
    state_file = os.path.join(_state_dir(), "handback-%s.json" % key)
    denies = 0
    try:
        with open(state_file) as f:
            denies = int(json.load(f).get("denies", 0))
    except Exception:
        pass
    if denies >= HANDBACK_MAX_DENIES:
        return
    try:
        os.makedirs(_state_dir(), exist_ok=True)
        with open(state_file, "w") as f:
            json.dump({"denies": denies + 1}, f)
    except Exception:
        return  # no counter, no deny: a deny without one could repeat forever
    return MSG_HANDBACK.format(n=len(text), cap=cap)


def g5_handback(data, ctx):
    if data.get("tool_name") != "SubagentHandback":
        return
    ti = data.get("tool_input")
    return check_handback(data, ti.get("message") if isinstance(ti, dict) else None)


def handoff_this_turn(path):
    """(handoff path, assistant text written after it) for the newest hand-off Write since the
    last human prompt, or (None, "")."""
    texts = []  # newest first
    for line in reverse_lines(path):
        rec = _loads(line)
        if not rec:
            continue
        if is_human_prompt(rec):
            break
        if rec.get("type") != "assistant":
            continue
        msg = rec.get("message")
        content = msg.get("content") if isinstance(msg, dict) else None
        if not isinstance(content, list):
            continue
        for b in reversed(content):
            if not isinstance(b, dict):
                continue
            if b.get("type") == "text" and isinstance(b.get("text"), str):
                texts.append(b["text"])
            elif b.get("type") == "tool_use" and b.get("name") == "Write":
                fp = (b.get("input") or {}).get("file_path")
                if isinstance(fp, str) and _HANDOFF_PATH.search(fp):
                    return fp, "\n".join(reversed(texts))
    return None, ""


def handoff_given(fp, text):
    """The reply holds the paste line and a markdown link to the file."""
    name = os.path.basename(fp)
    if f"Read {fp} and do it." not in text:
        return False
    return any(target.endswith(name) for target in re.findall(r"\]\(([^)\s]+)\)", text))


def g6_archive(data, ctx):
    if data.get("tool_name") != ARCHIVE_TOOL or not ctx.path:
        return
    ti = data.get("tool_input") if isinstance(data.get("tool_input"), dict) else {}
    if ti.get("session_id") not in (None, "self"):
        return
    fp, _ = handoff_this_turn(ctx.path)
    if fp and os.path.exists(fp):  # a hand-off deleted since is not owed to the user
        return MSG_HANDOFF_ARCHIVE.format(path=fp, name=os.path.basename(fp))


# --- Stop checks G10 to G12 -----------------------------------------------------

GIT_TIMEOUT = 2.0  # seconds for all git calls of G10 together
CLOCK_TURN_MIN = 10  # G11 applies to a turn of this many minutes or more
PLAN_TOOL = "mcp__ccd_turn__update_plan"
WRAP_TOOL = "mcp__ccd_turn__wrap_up"
EDIT_TOOLS = ("Edit", "Write", "NotebookEdit")
_GIT_COMMIT = re.compile(r"\bgit\b[^;&|\n]*?\bcommit\b")
_NO_PUSH = re.compile(
    r"do not push|don['’]t push|no push|local only|don['’]t commit|do not commit", re.I
)
_CLOCK = re.compile(r"\b\d{1,2}:\d{2}\b|\b\d+(?:\.\d+)?\s?(?:min(?:ute)?s?|h|hrs?|hours?)\b", re.I)
_ISO_TS = re.compile(r"(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)(?:\.\d+)?(Z|[+-]\d\d:?\d\d)?$")
_OPEN_STATUS = ("pending", "in_progress", "in-progress", "active", "todo")
_STEP_KEYS = ("content", "step", "title", "name", "text", "label", "description")
MSG_COMMIT = (
    "G10: this turn left work in {repo}: {n} uncommitted file(s), {m} unpushed commit(s). Rule 13: "
    "commit and push now, unless the user said not to. If the work is not finished, say what is left."
)
MSG_CLOCK = (
    "G11: this turn ran {n} min. Rule 12: give the clock as a Vienna time (HH:MM) and one estimate "
    "with a range."
)
MSG_OPEN_STEPS = (
    "G12: the task list still has open steps ({names}). Finish them now, or end with what is done, "
    "what is left, and one question."
)


def _timestamp(value):
    """Epoch seconds of an ISO-8601 timestamp string, or None."""
    m = _ISO_TS.match(value) if isinstance(value, str) else None
    if not m:
        return None
    y, mo, d, h, mi, s = (int(g) for g in m.groups()[:6])
    secs = calendar.timegm((y, mo, d, h, mi, s))
    tz = m.group(7)
    if tz and tz != "Z":
        off = int(tz[1:3]) * 3600 + int(tz[-2:]) * 60
        secs -= off if tz[0] == "+" else -off
    return secs


def _scan_record(rec, in_turn, t):
    """Fold one assistant record (newest block first) into the scan result `t`."""
    msg = rec.get("message")
    content = msg.get("content") if isinstance(msg, dict) else None
    if not isinstance(content, list):
        return
    for b in reversed(content):
        if not isinstance(b, dict):
            continue
        if b.get("type") == "text" and isinstance(b.get("text"), str):
            if in_turn and t["last_text"] is None:
                t["last_text"] = b["text"]
            continue
        if b.get("type") != "tool_use":
            continue
        name, inp = b.get("name"), b.get("input")
        inp = inp if isinstance(inp, dict) else {}
        if name == WRAP_TOOL:
            t["wrapped"] = t["wrapped"] or in_turn
            if not t["plan_done"]:
                t["plan"].append(("wrap", inp))
                t["plan_done"] = True
        elif name == PLAN_TOOL:
            if not t["plan_done"]:
                t["plan"].append(("plan", inp))
                t["plan_done"] = isinstance(inp.get("items"), list)
        elif not in_turn:
            continue
        elif name in EDIT_TOOLS:
            p = inp.get("file_path") or inp.get("notebook_path")
            if isinstance(p, str) and p:
                t["paths"].append(p)
        elif name == "Bash":
            if _GIT_COMMIT.search(str(inp.get("command") or "")):
                t["committed"] = True
        elif name == "AskUserQuestion":
            t["asked"] = True


def scan_turn(path):
    """One pass from the end of the transcript. Collects this turn's facts (since the last human
    prompt) and, going on past the prompt, the update_plan calls back to the call that set the
    list (or a wrap_up). Raises when the transcript cannot be read."""
    t = {"start": None, "paths": [], "committed": False, "asked": False, "wrapped": False,
         "last_text": None, "plan": [], "plan_done": False}
    in_turn = True
    for line in reverse_lines(path):
        if not in_turn:
            if t["plan_done"]:
                break  # the list is rebuilt: nothing older matters
            if b"mcp__ccd_turn__" not in line:
                continue
        rec = _loads(line)
        if not rec or rec.get("isSidechain"):
            continue
        if in_turn and is_human_prompt(rec):
            t["start"] = _timestamp(rec.get("timestamp"))
            in_turn = False
            continue
        if rec.get("type") == "assistant":
            _scan_record(rec, in_turn, t)
    return t


def _step_name(item):
    if isinstance(item, dict):
        for k in _STEP_KEYS:
            if isinstance(item.get(k), str) and item[k]:
                return item[k]
    return str(item)


def _step_status(item):
    s = str(item.get("status") or "pending").lower() if isinstance(item, dict) else "pending"
    return "in_progress" if s in ("in_progress", "in-progress", "active") else s


def _apply_plan(steps, inp):
    """Apply one update_plan input to `steps` (a list of [name, status])."""
    items = inp.get("items")
    if isinstance(items, list):
        steps[:] = [[_step_name(i), _step_status(i)] for i in items]

    def mark(n, status):
        if isinstance(n, int) and not isinstance(n, bool) and 1 <= n <= len(steps):
            steps[n - 1][1] = status

    done = inp.get("done")
    for n in done if isinstance(done, list) else [done]:
        mark(n, "completed")
    if inp.get("advance") is True:
        for s in steps:
            if s[1] == "in_progress":
                s[1] = "completed"
                break
        for s in steps:
            if s[1] == "pending":
                s[1] = "in_progress"
                break
    mark(inp.get("start"), "in_progress")


def open_steps(events):
    """Names of the steps still open after replaying the plan events (newest first)."""
    steps = []
    for kind, inp in reversed(events):
        if kind == "wrap":
            steps = []
        else:
            _apply_plan(steps, inp)
    return [name for name, status in steps if status in _OPEN_STATUS]


def _git(args, cwd, deadline):
    """Run git; returns stdout text, or None when it failed. Raises on timeout."""
    left = deadline - time.monotonic()
    if left <= 0:
        raise subprocess.TimeoutExpired("git", GIT_TIMEOUT)
    env = dict(os.environ, GIT_OPTIONAL_LOCKS="0", GIT_TERMINAL_PROMPT="0")
    r = subprocess.run(["git"] + args, cwd=cwd, capture_output=True, timeout=left, env=env)
    return r.stdout.decode("utf-8", "replace") if r.returncode == 0 else None


def said_no_push(path):
    """True when any human prompt of the session says not to push or commit."""
    for line in reverse_lines(path):
        if b'"user"' not in line:
            continue
        rec = _loads(line)
        if not rec or not is_human_prompt(rec):
            continue
        msg = rec.get("message")
        if _NO_PUSH.search(_text_of(msg.get("content") if isinstance(msg, dict) else None) or ""):
            return True
    return False


def g10_commit(data, t):
    """G10: this turn's work is uncommitted or unpushed in the session's repo."""
    cwd = data.get("cwd")
    if not isinstance(cwd, str) or not cwd or not (t["paths"] or t["committed"]):
        return
    deadline = time.monotonic() + GIT_TIMEOUT
    top = _git(["rev-parse", "--show-toplevel"], cwd, deadline)
    if top is None:
        return  # not a repo
    count = _git(["rev-list", "--count", "@{u}..HEAD"], cwd, deadline)  # fails without an upstream
    top = os.path.realpath(top.strip())
    unpushed = int(count.strip()) if t["committed"] and count is not None else 0
    dirty = 0
    if t["paths"]:
        status = _git(["status", "--porcelain", "-z", "-uall"], cwd, deadline)
        if status is None:
            return
        mine = {os.path.realpath(os.path.join(cwd, p)) for p in t["paths"]}
        parts = status.split("\0")
        i = 0
        while i < len(parts):
            entry = parts[i]
            i += 1
            if len(entry) < 4:
                continue
            if entry[0] in "RC":
                i += 1  # a rename or copy lists the old path next
            if os.path.realpath(os.path.join(top, entry[3:])) in mine:
                dirty += 1
    if not dirty and not unpushed:
        return
    if said_no_push(data["transcript_path"]):
        return
    return MSG_COMMIT.format(repo=os.path.basename(top), n=dirty, m=unpushed)


def _final_reply(data, t):
    last = data.get("last_assistant_message")
    return last if isinstance(last, str) else (t["last_text"] or "")


def g11_clock(data, t):
    """G11: a turn of 10 min or more ends without a clock time or a duration."""
    if t["start"] is None:
        return
    minutes = int((time.time() - t["start"]) // 60)
    if minutes >= CLOCK_TURN_MIN and not _CLOCK.search(_final_reply(data, t)):
        return MSG_CLOCK.format(n=minutes)


def g12_open_steps(data, t):
    """G12: task-list steps are still open and the turn asks nothing (quiet while a helper of this session still runs)."""
    if t["asked"] or t["wrapped"] or _final_reply(data, t).rstrip().endswith("?"):
        return
    try:
        with session_state(data) as state:
            if _live_spawns(state, time.time()):
                return  # a background helper runs; the app wakes the session when it ends
    except Exception:
        pass  # no state file, no slot
    names = open_steps(t["plan"])
    if names:
        return MSG_OPEN_STEPS.format(names=", ".join(names[:3]))


STOP_CHECKS = (("G10", g10_commit), ("G11", g11_clock), ("G12", g12_open_steps))


def g6_stop(data):
    fp, text = handoff_this_turn(data["transcript_path"])
    last = data.get("last_assistant_message")
    if isinstance(last, str) and last not in text:
        text = text + "\n" + last
    if fp and os.path.exists(fp) and not handoff_given(fp, text):
        return MSG_HANDOFF_REPLY.format(path=fp, name=os.path.basename(fp))


def stop(data):
    """Stop checks, first message wins: G6 (a hand-off written this turn must be in the reply), then
    G10 to G12 on the main thread. stop_hook_active skips all: one reminder per turn. Fail-open."""
    if data.get("stop_hook_active") or not data.get("transcript_path"):
        return
    try:
        msg = g6_stop(data)
    except Exception:
        msg = None
    if msg:
        return msg
    if not is_main_thread(data):
        return
    active = [(gid, fn) for gid, fn in STOP_CHECKS if not check_off(gid)]
    if not active:
        return
    try:
        t = scan_turn(data["transcript_path"])
    except Exception:
        return
    for gid, fn in active:
        try:
            msg = fn(data, t)
        except Exception:
            msg = None
        if msg:
            return msg


def subagentstop(data):
    if data.get("stop_hook_active"):
        spawn_done(data)
        return
    msg = check_handback(data, data.get("last_assistant_message"))
    if not msg:
        spawn_done(data)  # the helper is really done; a denied hand-back keeps its slot
    return msg


def scratch(data):
    """Allow a write only under a temp or scratchpad folder; None = allow."""
    ti = data.get("tool_input")
    ti = ti if isinstance(ti, dict) else {}
    path = ti.get("file_path") or ti.get("notebook_path")
    if not isinstance(path, str) or not path:
        return
    path = os.path.expanduser(path)
    cwd = data.get("cwd")
    if not os.path.isabs(path) and isinstance(cwd, str) and cwd:
        path = os.path.join(cwd, path)
    real = os.path.realpath(path)
    roots = list(SCRATCH_ROOTS)
    if os.environ.get("TMPDIR"):
        roots.append(os.environ["TMPDIR"])
    for root in roots:
        for r in {root.rstrip("/") or "/", os.path.realpath(root)}:
            if real == r or real.startswith(r.rstrip("/") + "/"):
                return
    if "/scratchpad/" in real or set(real.split("/")) & SCRATCH_DIRS:
        return
    return MSG_SCRATCH


def g7_heavy(data, ctx):
    """G7: refuse general-purpose, claude and an untyped Agent call (the owner chose deny, 2026-10-19)."""
    if data.get("tool_name") != "Agent" or not isinstance(data.get("tool_input"), dict):
        return
    kind = data["tool_input"].get("subagent_type") or "general-purpose"
    if kind in HEAVY_AGENTS:
        return MSG_HEAVY.format(kind=kind)


@contextlib.contextmanager
def session_state(data):
    """Locked read-modify-write of the per-session state file; the caller edits the yielded dict."""
    sid = re.sub(r"[^A-Za-z0-9_.-]", "_", str(data.get("session_id") or "unknown"))
    os.makedirs(_state_dir(), exist_ok=True)
    path = os.path.join(_state_dir(), sid + ".json")
    lock = open(path + ".lock", "a")
    try:
        if fcntl:
            fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            with open(path) as f:
                state = json.load(f)
        except Exception:
            state = {}
        if not isinstance(state, dict):
            state = {}
        yield state
        tmp = "%s.%d.tmp" % (path, os.getpid())
        with open(tmp, "w") as f:
            json.dump(state, f)
        os.replace(tmp, path)
    finally:
        lock.close()  # closing releases the lock


def _live_spawns(state, now):
    """Running slots as [{"id": tool_use_id or None, "t": start time}], oldest first.

    A slot written by the first version is a plain time; it loads as a slot without an id.
    """
    spawns = state.get("spawns")
    slots = []
    for s in spawns if isinstance(spawns, list) else []:
        if isinstance(s, dict):
            t, tid = s.get("t"), s.get("id")
        else:
            t, tid = s, None
        if isinstance(t, (int, float)) and not isinstance(t, bool) and now - t < SPAWN_TTL:
            slots.append({"id": tid, "t": t})
    return sorted(slots, key=lambda s: s["t"])


def g8_cap(data, ctx):
    """G8: helpers running at once, per session. An allowed call is recorded here, so keep this last."""
    if data.get("tool_name") != "Agent":
        return
    try:
        now = time.time()
        st = ctx.pace  # read before the lock: pace may take a moment
        with session_state(data) as state:
            spawns = _live_spawns(state, now)
            cap = CAPS[st["state"]]
            if len(spawns) >= cap:
                state["spawns"] = spawns
                return MSG_CAP.format(
                    n=len(spawns), helpers="helper" if len(spawns) == 1 else "helpers", cap=cap,
                    used=_pct(st.get("used")), budget=_pct(st.get("budget")), state=st["state"],
                )
            spawns.append({"id": data.get("tool_use_id"), "t": now})
            state["spawns"] = spawns
    except Exception:
        return  # no state, no cap: fail open


def spawn_done(data):
    """SubagentStop: the oldest running spawn of this session is done."""
    try:
        with session_state(data) as state:
            state["spawns"] = _live_spawns(state, time.time())[1:]
    except Exception:
        pass


def spawn_failed(data):
    """PostToolUseFailure on Agent: free the slot of that tool_use_id (a failed spawn never stops)."""
    tid = data.get("tool_use_id")
    if not tid or data.get("tool_name") not in (None, "Agent"):
        return
    try:
        with session_state(data) as state:
            spawns = _live_spawns(state, time.time())
            for i, s in enumerate(spawns):
                if s["id"] == tid:
                    del spawns[i]
                    break
            state["spawns"] = spawns
    except Exception:
        pass


def session_model(path):
    """`message.model` of the newest assistant record of a transcript (lowercase), or None."""
    if not path:
        return None
    try:
        for line in reverse_lines(path):
            if b'"assistant"' not in line:
                continue
            rec = _loads(line)
            msg = rec.get("message") if rec and rec.get("type") == "assistant" else None
            model = msg.get("model") if isinstance(msg, dict) else None
            if isinstance(model, str) and model and not model.startswith("<"):  # skip "<synthetic>"
                return model.lower()
    except Exception:
        pass
    return None


_ENTRYPOINT = re.compile(rb'"entrypoint"\s*:\s*"([^"]*)"')


def session_entrypoint(path, max_lines=200):
    """The `entrypoint` field of the first transcript records ("cli", "claude-desktop", ...), or None."""
    if not path:
        return None
    try:
        with open(path, "rb") as f:
            for i, line in enumerate(f):
                if i >= max_lines:
                    break
                m = _ENTRYPOINT.search(line)
                if m:
                    return m.group(1).decode("utf-8", "replace")
    except Exception:
        pass
    return None


def g9_plan_gate(data, ctx):
    """G9: planning skills only in plan mode; no plan mode when the week runs ahead."""
    tool = data.get("tool_name")
    if tool == "Skill":
        if _tool_input(data).get("skill") not in PLAN_SKILLS or not is_main_thread(data):
            return
        if data.get("permission_mode") == "plan" or ctx.pace["state"] in ("ahead", "critical"):
            return
        # The main model decides: Opus and Fable already plan on Opus. Only Sonnet or Haiku is gated,
        # and only where the entrypoint is known (the CLI has opusplan; the desktop app has none).
        rank = _model_rank(session_model(ctx.path))
        if rank is None or rank >= MODEL_RANK["opus"]:
            return
        entry = session_entrypoint(ctx.path)
        if entry == "cli":
            return MSG_PLAN_GATE
        if entry == "claude-desktop":
            return MSG_PLAN_GATE_APP
    if tool == "EnterPlanMode" and ctx.pace["state"] in ("ahead", "critical"):
        st = ctx.pace
        return MSG_NO_PLAN_MODE.format(state=st["state"], used=_pct(st.get("used")), budget=_pct(st.get("budget")))


MODEL_RANK = {"haiku": 0, "sonnet": 1, "opus": 2, "fable": 3}


def _model_rank(model):
    """Cost rank of a model name or id (haiku < sonnet < opus < fable); None when unknown."""
    name = str(model or "").lower()
    for word, rank in MODEL_RANK.items():
        if word in name:
            return rank
    return None


def route_agent(data, ctx):
    """Router: (new tool_input or None, note or None) for a main-thread Agent call.

    Plan and a final code-reviewer: opus + high (sonnet + high when the week is ahead). Every other
    row never raises cost: the lower of the caller's model and the table's (Explore,
    claude-code-guide, statusline-setup: haiku; the rest: sonnet), effort high unless the caller
    asked for low or medium, none for haiku. xhigh and max never pass. `fork` ignores model, so it
    is left alone.
    """
    if data.get("tool_name") != "Agent" or not is_main_thread(data) or not isinstance(data.get("tool_input"), dict):
        return None, None
    ti = data["tool_input"]
    kind = ti.get("subagent_type")
    if kind == "fork":
        return None, None
    desc = str(ti.get("description") or "")
    head = str(ti.get("prompt") or "")[:400]
    final = kind == "code-reviewer" and bool(_FINAL_REVIEW.search(desc) or _FINAL_REVIEW.search(head))
    opus_row = kind == "Plan" or (final and not _PANEL_ANGLE.search(desc))
    note = None
    if opus_row:  # table-driven: opus + high, sonnet + high when the week is ahead
        model, effort = "opus", "high"
        if ctx.pace["state"] in ("ahead", "critical"):
            model = "sonnet"
            note = "the week is %s, so opus became sonnet" % ctx.pace["state"]
    else:  # never raise cost: the lower of the caller's model and the table's
        table = "haiku" if kind in LOOKUP_AGENTS else "sonnet"
        model = table
        given = ti.get("model")
        given_rank = _model_rank(given)
        if given_rank is not None and given_rank <= MODEL_RANK[table]:
            model = given
        effort = "high"
        if ti.get("effort") in ("low", "medium"):
            effort = ti["effort"]
    new = dict(ti)
    new["model"] = model
    if _model_rank(model) == 0:  # haiku takes no effort
        new.pop("effort", None)
    else:
        new["effort"] = effort
    if new == ti:
        return None, None
    shown = new.get("effort")
    msg = "guard router: helper runs on %s%s" % (model, " (%s)" % shown if shown else "")
    return new, msg + (" - " + note if note else "")


def browser_calls_this_turn(path, tool_use_id):
    """Browser tool_use blocks since the last human prompt, not counting the current call."""
    n = 0
    for line in reverse_lines(path):
        if b'"user"' not in line and b"mcp__C" not in line and b"mcp__c" not in line:
            continue
        rec = _loads(line)
        if not rec:
            continue
        if is_human_prompt(rec):
            break
        if rec.get("type") != "assistant":
            continue
        content = (rec.get("message") or {}).get("content") if isinstance(rec.get("message"), dict) else None
        for b in content if isinstance(content, list) else []:
            if (
                isinstance(b, dict)
                and b.get("type") == "tool_use"
                and str(b.get("name") or "").startswith(BROWSER_PREFIXES)
                and b.get("id") != tool_use_id
            ):
                n += 1
    return n


def w3_browser(data, ctx):
    """W3: many browser calls in a main window this turn: route checks to ui-verifier."""
    if not str(data.get("tool_name") or "").startswith(BROWSER_PREFIXES):
        return
    if data.get("agent_id") or not ctx.path or "/subagents/" in ctx.path:
        return  # a subagent (ui-verifier) is where browser work belongs
    n = browser_calls_this_turn(ctx.path, data.get("tool_use_id"))
    if n == BROWSER_WARN_AT or (n > BROWSER_WARN_AT and (n - BROWSER_WARN_AT) % BROWSER_WARN_EVERY == 0):
        return MSG_BROWSER.format(n=n + 1)


def session_spawns(path):
    """(Agent tool_use blocks so far, sum of totalTokens the finished Agent results report).

    A foreground agent reports in its tool result (toolUseResult.totalTokens); a background one in a
    task-notification attachment (attachment.usage.totalTokens), counted once per task id.
    """
    spawns, tokens, tasks = 0, 0, set()
    with open(path, "rb") as f:
        for line in f:
            if b'"Agent"' in line:
                msg = (_loads(line) or {}).get("message")
                content = msg.get("content") if isinstance(msg, dict) else None
                for b in content if isinstance(content, list) else []:
                    if isinstance(b, dict) and b.get("type") == "tool_use" and b.get("name") == "Agent":
                        spawns += 1
            if b'"totalTokens"' in line and b'"toolUseResult"' in line:
                rec = _loads(line)
                tur = (rec or {}).get("toolUseResult")
                if isinstance(tur, dict) and _num(tur.get("totalTokens")):
                    tokens += int(tur["totalTokens"])
            if b'"totalTokens"' in line and b"task-notification" in line:
                att = (_loads(line) or {}).get("attachment")
                usage = att.get("usage") if isinstance(att, dict) else None
                if isinstance(usage, dict) and _num(usage.get("totalTokens")):
                    m = re.search(r"<task-id>([^<]+)</task-id>", str(att.get("prompt") or ""))
                    key = m.group(1) if m else att.get("delivery_id") or len(tasks)
                    if key not in tasks:
                        tasks.add(key)
                        tokens += int(usage["totalTokens"])
    return spawns, tokens


def w4_budget(data, ctx):
    """W4: past the spawn or token cap, a budget note on every 5th spawn."""
    if data.get("tool_name") != "Agent" or not ctx.path:
        return
    spawns, tokens = session_spawns(ctx.path)
    prior = spawns
    if data.get("tool_use_id"):
        prior = max(0, spawns - 1) if _has_tool_use(ctx.path, data.get("tool_use_id")) else spawns
    over = prior >= SPAWN_CAP or tokens >= TOKEN_CAP
    if over and prior % BUDGET_EVERY == 0:
        return MSG_BUDGET.format(n=prior, tok=tokens // 1000, cap=SPAWN_CAP, tcap=TOKEN_CAP // 1000)


def _has_tool_use(path, tool_use_id):
    """The current call is already in the transcript (written before the hook runs)."""
    needle = ('"id":"%s"' % tool_use_id).encode()
    needle2 = ('"id": "%s"' % tool_use_id).encode()
    for i, line in enumerate(reverse_lines(path)):
        if needle in line or needle2 in line:
            return True
        if i > 50:
            return False
    return False


# --- G13 silence note, G14 sed -i, G15 secret reads ---------------------------

G13_AFTER = 8  # tool calls in a row with no text before the note
MSG_G13 = "G13: 8 tool calls with no text. Write one short status line for the user now, then go on."
MSG_G14 = (
    "G14: BSD sed -i on macOS differs from GNU sed and breaks files. Use the Edit tool, or "
    "python3 for bulk edits."
)
MSG_G15_FILE = (
    "G15: never read local.zsh: it holds API keys. Use `test -f` or `git check-ignore` for facts "
    "about it, or ask the user."
)
MSG_G15_SEARCH = (
    "G15: this search can reach local.zsh (API keys). Exclude it: `rg -g '!local.zsh' ...` or "
    "`grep --exclude=local.zsh ...`."
)
MSG_G15_PS = (
    "G15: this prints full command lines, which can hold API keys. Use `pgrep -f <name>` for pids, "
    "or `ps -o pid=,etime=,ucomm= -p <pid>`."
)
SECRET_FILE = "~/.config/zsh/local.zsh"
_WORDS = re.compile(r"\w{2,}")


def g13_streak(path, tool_use_id=None):
    """Tool calls since the last assistant text with real words (or the last human prompt).

    Reads the transcript from the end and stops at the text or at G13_AFTER + 1. With a
    tool_use_id the count ends at that call, so parallel calls each see their own number.
    """
    n = 0
    active = not tool_use_id
    for line in reverse_lines(path):
        if b'"tool_result"' in line:
            continue  # a tool result is never a prompt or an assistant text
        if b'"assistant"' not in line and b'"user"' not in line:
            continue
        rec = _loads(line)
        if not rec or rec.get("isSidechain"):
            continue
        if rec.get("type") == "user":
            if is_human_prompt(rec):
                break
            continue
        if rec.get("type") != "assistant":
            continue
        msg = rec.get("message")
        content = msg.get("content") if isinstance(msg, dict) else None
        for b in reversed(content) if isinstance(content, list) else ():
            if not isinstance(b, dict):
                continue
            if b.get("type") == "tool_use":
                if not active:
                    active = b.get("id") == tool_use_id
                if active:
                    n += 1
            elif b.get("type") == "text" and active:
                text = b.get("text")
                if isinstance(text, str) and _WORDS.search(text):
                    return n
        if n > G13_AFTER:
            break
    return n


def posttool(data):
    """G13 (PostToolUse, main thread): one JSON line after the 8th tool call with no text, else None."""
    try:
        if check_off("G13") or os.environ.get("CLAUDE_CODE_ENTRYPOINT") == "claude-desktop":
            return None  # the desktop app has its own nudge and its transcript can miss text blocks
        if not is_main_thread(data):
            return None
        path = data.get("transcript_path")
        if not isinstance(path, str) or not path:
            return None
        tid = data.get("tool_use_id")
        if g13_streak(path, tid if isinstance(tid, str) and tid else None) == G13_AFTER:
            return json.dumps({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": MSG_G13}})
    except Exception:
        pass
    return None


# One shared Bash command splitter for G14 and G15.
_ASSIGN = re.compile(r"[A-Za-z_][A-Za-z0-9_]*=")
_HEREDOC = re.compile(r"(?<!<)<<-?\s*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\1")
_RESERVED = {"if", "then", "else", "elif", "do", "while", "until", "{", "}", "!"}
_SHELLS = {"bash", "sh", "zsh", "dash", "ksh"}
_EXEC_FLAGS = ("-exec", "-execdir", "-ok", "-okdir")
# wrapper command -> its options that take a value
_WRAPPERS = {
    "sudo": {"-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-U"},
    "doas": {"-u", "-C"},
    "env": {"-u", "-C", "-S"},
    "xargs": {"-I", "-n", "-P", "-L", "-s", "-d", "-E", "-a", "-J"},
    "nice": {"-n"},
    "timeout": {"-s", "-k"},
    "exec": {"-a"},
    "command": set(),
    "builtin": set(),
    "nohup": set(),
    "time": set(),
}


def _strip_heredocs(cmd):
    out, pending = [], []
    for line in cmd.split("\n"):
        if pending:
            if line.strip() == pending[0]:
                pending.pop(0)
            continue
        out.append(line)
        pending += [m.group(2) for m in _HEREDOC.finditer(line)]
    return "\n".join(out)


def _segments(cmd):
    """Split a command line at ; | & newline ( ) ` and $( outside quotes. Quotes stay in the text."""
    segs, cur, quote, i, n = [], [], None, 0, len(cmd)
    while i < n:
        c = cmd[i]
        if quote:
            cur.append(c)
            if quote == '"' and c == "\\" and i + 1 < n:
                cur.append(cmd[i + 1])
                i += 1
            elif c == quote:
                quote = None
        elif c == "\\" and i + 1 < n:
            cur.append(c)
            cur.append(cmd[i + 1])
            i += 1
        elif c in "'\"":
            quote = c
            cur.append(c)
        elif c in ";|&\n()`" or (c == "$" and cmd[i + 1 : i + 2] == "("):
            segs.append("".join(cur))
            cur = []
            if c == "$":
                i += 1
        else:
            cur.append(c)
        i += 1
    segs.append("".join(cur))
    return segs


def _peel(tokens):
    """Drop env assignments, reserved words and wrappers (sudo, env, xargs ...) from the front."""
    for _ in range(8):
        i = 0
        while i < len(tokens) and (tokens[i] in _RESERVED or _ASSIGN.match(tokens[i])):
            i += 1
        tokens = tokens[i:]
        if not tokens:
            return tokens
        optval = _WRAPPERS.get(os.path.basename(tokens[0]))
        if optval is None:
            return tokens
        i = 1
        while i < len(tokens) and tokens[i].startswith("-") and tokens[i] != "-":
            if tokens[i] == "--":
                i += 1
                break
            i += 2 if tokens[i] in optval else 1
        if os.path.basename(tokens[0]) == "timeout":
            i += 1  # the duration
        tokens = tokens[i:]
    return tokens


def _abspath(p, cwd):
    p = os.path.expanduser(os.path.expandvars(p))
    if not os.path.isabs(p) and cwd:
        p = os.path.join(cwd, p)
    return os.path.realpath(p)


def _walk(cmd, cwd, depth=0):
    """Yield (tokens, extra, cwd) for each simple command of a Bash command line.

    tokens start at the command word (wrappers peeled). Commands inside `bash -c`, `eval` and
    `find -exec` are yielded too (extra = the find words before -exec). cwd follows `cd`.
    """
    for seg in _segments(_strip_heredocs(cmd)):
        try:
            toks = _peel(shlex.split(seg))
        except ValueError:
            continue  # unbalanced quotes: not a command Bash would run
        if not toks:
            continue
        name = os.path.basename(toks[0])
        if name in ("cd", "pushd"):
            dirs = [t for t in toks[1:] if not t.startswith("-")]
            if dirs:
                cwd = _abspath(dirs[0], cwd)
        yield toks, (), cwd
        if depth >= 3:
            continue
        if name in _SHELLS:
            for k, t in enumerate(toks[1:], 1):
                if re.fullmatch(r"-[A-Za-z]*c", t) and k + 1 < len(toks):
                    yield from _walk(toks[k + 1], cwd, depth + 1)
                    break
        elif name == "eval":
            yield from _walk(" ".join(toks[1:]), cwd, depth + 1)
        elif name == "find":
            for j, t in enumerate(toks):
                if t in _EXEC_FLAGS:
                    sub = []
                    for u in toks[j + 1 :]:
                        if u in (";", "+"):
                            break
                        sub.append(u)
                    sub = _peel(sub)
                    if sub:
                        yield sub, tuple(toks[1:j]), cwd


def _bash_command(data, tool="Bash"):
    if data.get("tool_name") != tool:
        return None
    cmd = _tool_input(data).get("command")
    return cmd if isinstance(cmd, str) and cmd else None


def _sed_in_place(args):
    """True when sed's arguments hold -i, -i.bak, -ni, -Ei, --in-place[=SUFFIX]."""
    skip = False
    for a in args:
        if skip:
            skip = False
        elif a == "--":
            break
        elif a.startswith("--"):
            if a == "--in-place" or a.startswith("--in-place="):
                return True
            skip = a in ("--expression", "--file", "--line-length")
        elif a.startswith("-") and len(a) > 1:
            for k, c in enumerate(a[1:], 1):
                if c == "i":
                    return True
                if c in "efl":  # the rest of this word, or the next word, is its value
                    skip = k == len(a) - 1
                    break
    return False


def g14_sed(data, ctx):
    """G14: deny BSD `sed -i` on macOS. Quoted text, `gsed` and `sed -n` pass."""
    cmd = _bash_command(data)
    if not cmd or sys.platform != "darwin" or "sed" not in cmd or check_off("G14"):
        return
    try:
        for toks, _extra, _cwd in _walk(cmd, data.get("cwd")):
            if os.path.basename(toks[0]) == "sed" and _sed_in_place(toks[1:]):
                return MSG_G14
    except Exception:
        return


SECRET_READERS = {
    "cat", "bat", "batcat", "less", "more", "most", "head", "tail", "rg", "grep", "egrep", "fgrep", "ag",
    "ack", "sed", "gsed", "awk", "gawk", "mawk", "cut", "strings", "xxd", "hexdump", "od", "nl", "tac",
    "rev", "sort", "uniq", "paste", "fold", "column", "cp", "mv", "scp", "rsync", "open", "code", "cursor",
    "vim", "vi", "nvim", "nano", "view", "pbcopy", "base64", "jq", "yq", "diff", "cmp", "ln", "install",
}
SECRET_SEARCHERS = {"rg", "grep", "egrep", "fgrep", "ag", "ack"}  # also read every file under a folder
_GLOB = re.compile(r"[*?\[]")
_ARGS_FIELDS = {"args", "command", "cmd", "command_line", "cmdline"}
_PS_VALUE_LONG = {
    "--format", "--pid", "--ppid", "--sid", "--tty", "--user", "--group", "--sort", "--cols", "--columns",
    "--width", "--lines", "--rows",
}


def _secret_target():
    raw = os.path.normpath(os.path.expanduser(SECRET_FILE))
    return raw, os.path.realpath(raw)


def _cf(s):
    """Case-fold a path on macOS, where the usual volume ignores letter case."""
    return s.lower() if sys.platform == "darwin" else s


def _is_secret(path, cwd, target, dir_ok=False):
    """True when `path` (word as typed) is the secret file, a glob that matches it, or (dir_ok) its folder."""
    raw, real = _cf(target[0]), _cf(target[1])
    p = os.path.expanduser(os.path.expandvars(path))
    if not p or p.startswith("-"):
        return False
    if not os.path.isabs(p) and cwd:
        p = os.path.join(cwd, p)
    if _GLOB.search(p):
        d, base = os.path.split(os.path.normpath(p))
        return fnmatch.fnmatchcase(raw, _cf(os.path.normpath(p))) or fnmatch.fnmatchcase(
            real, _cf(os.path.join(os.path.realpath(d), base))
        )
    got = _cf(os.path.realpath(p))
    return got == real or (dir_ok and got == os.path.dirname(real))


def _ps_leaks(args):
    """True for a ps call that lists processes with their command lines (aux, -ef, -o args, ww, eww).

    On macOS every ps without an -o field list leaks (its CMD column is the full command line),
    unless -c is given; -O only adds columns, so it does not count as a list.
    """
    flags, bare, fmt, i, n = {}, set(), [], 0, len(args)
    has_o = False
    while i < n:
        a = args[i]
        i += 1
        if a.startswith("--"):
            if a.startswith("--format="):
                fmt.append(a[9:])
                has_o = True
            elif a in _PS_VALUE_LONG and i < n:
                if a == "--format":
                    fmt.append(args[i])
                    has_o = True
                i += 1
        elif a.startswith("-") and len(a) > 1:
            for k, c in enumerate(a[1:], 1):
                if c in "oOpCtuUgGsD":  # takes a value: the rest of the word or the next word
                    val = a[k + 1 :]
                    if not val and i < n:
                        val = args[i]
                        i += 1
                    if c in "oO":
                        fmt.append(val)
                        has_o = has_o or c == "o"
                    break
                flags[c] = flags.get(c, 0) + 1
        elif re.fullmatch(r"[A-Za-z]+", a):  # BSD style flags without a dash
            for c in a:
                flags[c] = flags.get(c, 0) + 1
                bare.add(c)
    for val in fmt:
        if any(f.split("=")[0].lower() in _ARGS_FIELDS for f in re.split(r"[,\s]+", val) if f):
            return True
    if flags.get("w", 0) >= 2 or "f" in flags or "e" in bare or "u" in bare:
        return True
    if not fmt and any(c in flags for c in "eAaxd"):
        return True
    if sys.platform == "darwin":
        return not has_o and "c" not in flags and not {"--help", "--version"} & set(args)
    return False


def _pgrep_leaks(args):
    """True for pgrep -fl / -af / -lf / --list-full (they print full command lines)."""
    flags, i, n = set(), 0, len(args)
    while i < n:
        a = args[i]
        i += 1
        if a == "--list-full":
            return True
        if a.startswith("--") or not a.startswith("-") or len(a) < 2:
            continue
        for k, c in enumerate(a[1:], 1):
            if c in "dFgGLPrstuU":  # takes a value
                if k == len(a) - 1:
                    i += 1
                break
            flags.add(c)
    return "f" in flags and ("l" in flags or "a" in flags)


# Recursive searches: the options of each searcher that take a value, and the ones G15 looks at.
_SEARCH_VAL = {
    "grep": ("efmABCdD", {
        "--regexp", "--file", "--max-count", "--after-context", "--before-context", "--context",
        "--directories", "--devices", "--include", "--exclude", "--exclude-dir", "--exclude-from",
        "--label", "--binary-files", "--group-separator",
    }),
    "rg": ("efgmABCdjMrtTE", {
        "--regexp", "--file", "--glob", "--iglob", "--max-count", "--after-context", "--before-context",
        "--context", "--max-depth", "--maxdepth", "--threads", "--max-columns", "--replace", "--type",
        "--type-not", "--type-add", "--type-clear", "--ignore-file", "--encoding", "--colors", "--color",
        "--sort", "--sortr", "--path-separator", "--pre", "--pre-glob", "--engine", "--dfa-size-limit",
        "--regex-size-limit", "--max-filesize", "--context-separator", "--field-context-separator",
        "--field-match-separator", "--hostname-bin", "--hyperlink-format",
    }),
    "ag": ("ABCGgmpW", {
        "--ignore", "--ignore-dir", "--depth", "--file-search-regex", "--path-to-ignore", "--pager",
        "--after", "--before", "--context", "--max-count", "--width", "--workers",
    }),
    "ack": ("ABCmgt", {
        "--ignore-dir", "--noignore-dir", "--ignore-file", "--type", "--after-context", "--before-context",
        "--context", "--max-count", "--output", "--pager",
    }),
}


def _search_short(fam, c, val, o):
    """Fold one short option (val is None for a flag) into the search options `o`."""
    if fam in ("grep", "rg") and c in "ef" and val is not None:
        o["pat"] = True
    elif fam == "grep":
        if c in "rR" and val is None or c == "d" and val == "recurse":
            o["rec"] = True
    elif fam in ("rg", "ag"):
        if c == "g" and val is not None:
            o["globs"].append(val)
            o["pat"] = o["pat"] or fam == "ag"  # ag -g PATTERN replaces the content pattern
        elif val is None and (c == "u" or (c == "." and fam == "rg") or (c == "U" and fam == "ag")):
            o["unres"] = True
    elif fam == "ack" and c in "gf":
        o["pat"] = True


def _search_long(fam, key, val, o):
    """Fold one long option (value already split off) into the search options `o`."""
    if fam in ("grep", "rg") and key in ("--regexp", "--file"):
        o["pat"] = True
    elif fam == "grep":
        if key in ("--recursive", "--dereference-recursive") or (key == "--directories" and val == "recurse"):
            o["rec"] = True
        elif key == "--include":
            o["inc"].append(val)
        elif key == "--exclude":
            o["xfile"].append(val)
        elif key == "--exclude-dir":
            o["xdir"].append(val)
    elif fam == "rg":
        if key == "--files":
            o["pat"] = True
        elif key in ("--glob", "--iglob"):
            o["globs"].append(val)
        elif key in ("--hidden", "--unrestricted") or (key.startswith("--no-ignore") and key != "--no-ignore-messages"):
            o["unres"] = True
    elif fam == "ag":
        if key == "--ignore":
            o["globs"].append("!" + val)
        elif key == "--ignore-dir":
            o["xdir"].append(val)
        elif key in ("--unrestricted", "--skip-vcs-ignores", "--hidden"):
            o["unres"] = True
        elif key == "--file-search-regex":
            o["pat"] = True
    elif fam == "ack" and key == "--ignore-dir":
        o["xdir"].append(val)


def _search_opts(fam, args):
    """Parse the arguments of grep, rg, ag or ack: path operands, recursion, unrestricted flags, excludes."""
    short_val, long_val = _SEARCH_VAL[fam]
    o = {"pos": [], "pat": False, "rec": fam == "ack", "unres": False,
         "globs": [], "inc": [], "xfile": [], "xdir": []}
    i, n = 0, len(args)
    while i < n:
        a = args[i]
        i += 1
        if a == "--":
            o["pos"] += args[i:]
            break
        if a.startswith("--"):
            key, eq, val = a.partition("=")
            if not eq and key in long_val and i < n:
                val = args[i]
                i += 1
            _search_long(fam, key, val, o)
        elif a.startswith("-") and len(a) > 1:
            for k, c in enumerate(a[1:], 1):
                if c in short_val:
                    val = a[k + 1 :]
                    if not val and i < n:
                        val = args[i]
                        i += 1
                    _search_short(fam, c, val, o)
                    break
                _search_short(fam, c, None, o)
        else:
            o["pos"].append(a)
    return o


def _glob_hits(g, dirs, fname, path):
    """True when an ignore-style glob (rg -g '!GLOB') matches the file, one of its folders, or its path."""
    g = _cf(g).lstrip("/")
    only_dir = g.endswith("/")
    g = g.rstrip("/")
    if not g:
        return False
    names = dirs if only_dir else dirs + [fname]
    if any(fnmatch.fnmatchcase(c, g) for c in names):
        return True
    return any(fnmatch.fnmatchcase(path, p) for p in (g, "*/" + g, "*/" + g + "/*"))


def _search_excluded(o, path, dirs, fname):
    """True when the search options leave `path` out (--exclude, --exclude-dir, --include, rg/ag globs)."""
    path = _cf(path)
    if any(fnmatch.fnmatchcase(fname, _cf(g)) or fnmatch.fnmatchcase(path, _cf(g)) for g in o["xfile"]):
        return True
    if any(fnmatch.fnmatchcase(d, _cf(g).rstrip("/")) for g in o["xdir"] for d in dirs):
        return True
    if o["inc"] and not any(fnmatch.fnmatchcase(fname, _cf(g)) for g in o["inc"]):
        return True
    keep = [g for g in o["globs"] if not g.startswith("!")]
    if any(_glob_hits(g[1:], dirs, fname, path) for g in o["globs"] if g.startswith("!")):
        return True
    if keep:
        return not any(
            fnmatch.fnmatchcase(fname, _cf(g)) or fnmatch.fnmatchcase(path, _cf(g)) or fnmatch.fnmatchcase(path, "*/" + _cf(g))
            for g in keep
        )
    return False


def _search_reaches(o, root, cand):
    """True when the search root covers `cand` (the secret file) and no option leaves it out."""
    root, cand = os.path.normpath(root), _cf(cand)
    base = root
    if _GLOB.search(root):  # a glob operand: the part before the first glob word is the base folder
        keep = []
        for part in root.split(os.sep):
            if _GLOB.search(part):
                break
            keep.append(part)
        base = os.sep.join(keep) or os.sep
        pat = _cf(root)
        if not (fnmatch.fnmatchcase(cand, pat) or fnmatch.fnmatchcase(cand, pat.rstrip(os.sep) + os.sep + "*")):
            return False
    elif not (_cf(root) == cand or cand.startswith(_cf(root).rstrip(os.sep) + os.sep)):
        return False
    base = _cf(base)
    if not cand.startswith(base.rstrip(os.sep) + os.sep):
        return True
    parts = cand[len(base.rstrip(os.sep)) + 1 :].split(os.sep)
    return not _search_excluded(o, cand, parts[:-1], parts[-1])


def _search_leaks(name, args, cwd, target):
    """True for a recursive search (grep -r, ack, and rg/ag with ignore rules off) that reaches the secret file."""
    fam = "grep" if name in ("grep", "egrep", "fgrep") else name
    o = _search_opts(fam, args)
    if (fam == "grep" and not o["rec"]) or (fam in ("rg", "ag") and not o["unres"]):
        return False  # rg and ag skip the hidden, git-ignored file unless told not to
    paths = [p for p in (o["pos"] if o["pat"] else o["pos"][1:]) if p != "-"]
    cwd = cwd or os.getcwd()
    for p in paths or [cwd]:
        q = os.path.expanduser(os.path.expandvars(p))
        if not os.path.isabs(q):
            q = os.path.join(cwd, q)
        for root in {os.path.normpath(q), os.path.realpath(q)}:
            if any(_search_reaches(o, root, cand) for cand in target):
                return True
    return False


def _bash_leaks(cmd, cwd):
    """The G15 message for a Bash command that reads local.zsh or prints command lines, else None."""
    target = _secret_target()
    for toks, extra, ccwd in _walk(cmd, cwd):
        name = os.path.basename(toks[0])
        args = toks[1:]
        if name == "procs" or (name == "ps" and _ps_leaks(args)) or (name == "pgrep" and _pgrep_leaks(args)):
            return MSG_G15_PS
        for k, t in enumerate(toks):  # input redirect: cmd < file
            if t == "<" and k + 1 < len(toks) and _is_secret(toks[k + 1], ccwd, target):
                return MSG_G15_FILE
            if t.startswith("<") and not t.startswith(("<<", "<(")) and len(t) > 1 and _is_secret(t[1:], ccwd, target):
                return MSG_G15_FILE
        if name == "dd" and any(a.startswith("if=") and _is_secret(a[3:], ccwd, target) for a in args[:200]):
            return MSG_G15_FILE
        if name in SECRET_READERS:
            if any(_is_secret(a, ccwd, target) for a in args[:200]):
                return MSG_G15_FILE
            if name in SECRET_SEARCHERS and (
                any(_is_secret(a, ccwd, target, True) for a in args[:200]) or _search_leaks(name, args[:200], ccwd, target)
            ):
                return MSG_G15_SEARCH
            if any(_is_secret(a, ccwd, target, True) for a in extra[:200]):
                return MSG_G15_FILE
    return None


def g15_secrets(data, ctx):
    """G15: deny reading local.zsh (Read, Grep or Bash) and printing full process command lines."""
    tool = data.get("tool_name")
    if tool not in ("Read", "Grep", "Bash") or check_off("G15"):
        return
    try:
        cwd = data.get("cwd") if isinstance(data.get("cwd"), str) and data.get("cwd") else os.getcwd()
        if tool == "Bash":
            cmd = _bash_command(data)
            return _bash_leaks(cmd, cwd) if cmd else None
        key = "file_path" if tool == "Read" else "path"
        path = _tool_input(data).get(key)
        if isinstance(path, str) and path and _is_secret(path, cwd, _secret_target(), dir_ok=tool == "Grep"):
            return MSG_G15_FILE
    except Exception:
        return


def pretool(data, ctx=None):
    ctx = ctx or Ctx(data)
    # g8_cap records the spawn it allows, so it runs after every other deny
    for guard in (
        g3_pretool, g7_heavy, g1_agent, g2_polling, g4_ask, g5_handback, g6_archive, g9_plan_gate,
        g14_sed, g15_secrets, g8_cap,
    ):
        msg = guard(data, ctx)
        if msg:
            return msg


WARNERS = (w3_browser, w4_budget)


def pretool_warn(data, ctx=None):
    """Router and warnings for an allowed call: one JSON line for stdout, or None.

    The line holds additionalContext, plus permissionDecision allow and updatedInput (the whole
    tool input, model and effort replaced) when the router changed the call.
    """
    ctx = ctx or Ctx(data)
    notes = []
    updated = None
    try:
        updated, note = route_agent(data, ctx)
        if note:
            notes.append(note)
    except Exception:
        updated = None
    for warner in WARNERS:
        try:
            msg = warner(data, ctx)
        except Exception:
            msg = None
        if msg:
            notes.append(msg)
    if not notes and not updated:
        return None
    hso = {"hookEventName": "PreToolUse"}
    if updated:
        hso["permissionDecision"] = "allow"
        hso["updatedInput"] = updated
    hso["additionalContext"] = "\n".join(notes)
    return json.dumps({"hookSpecificOutput": hso})


def pace_note(st, last):
    """The note for a pace state that was not announced yet (last = the state announced before)."""
    state = st["state"]
    args = {"used": _pct(st.get("used")), "budget": _pct(st.get("budget"))}
    if state == "critical":  # on every prompt while critical
        return MSG_PACE_CRITICAL.format(**args)
    if state == "ahead" and last != "ahead":  # once
        return MSG_PACE_AHEAD.format(**args)


def prompt(data, ctx=None):
    """G3 nudge and pace notes: print JSON for stdout (or None); counters live in TMPDIR."""
    ctx = ctx or Ctx(data)
    if pace is not None:
        try:
            pace.refresh(budget_s=5)
        except Exception:
            pass
    st = ctx.pace
    note = None
    count = 0
    try:
        with session_state(data) as state:
            try:
                count = int(state.get("count", 0))
            except (TypeError, ValueError):
                count = 0
            count = count + 1 if ctx.size >= NUDGE_CONTEXT else 0
            state["count"] = count
            if st["state"] != "unknown":  # a gap in the data does not reset what was announced
                note = pace_note(st, state.get("pace"))
                state["pace"] = st["state"]
    except Exception:
        count = 1 if ctx.size >= NUDGE_CONTEXT else 0
    notes = []
    if count >= 1 and (count - 1) % NUDGE_EVERY == 0:
        notes.append(
            f"Context is {ctx.size // 1000}K. Keep PROGRESS.md (or the run's ledger) current; "
            "auto-compact carries the run. Hand off only when the user asks."
        )
    if note:
        notes.append(note)
    if notes:
        return json.dumps(
            {"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": "\n".join(notes)}}
        )


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode not in ("pretool", "prompt", "subagentstop", "posttoolfailure", "posttool", "scratch", "stop"):
        return 0
    data = json.loads(sys.stdin.read())
    if not isinstance(data, dict):
        return 0
    if mode == "posttoolfailure":
        spawn_failed(data)
        return 0
    if mode == "posttool":
        out = posttool(data)
        if out:
            sys.stdout.write(out + "\n")
        return 0
    if mode in ("pretool", "subagentstop", "scratch", "stop"):
        ctx = Ctx(data)
        handlers = {"pretool": lambda d: pretool(d, ctx), "subagentstop": subagentstop, "scratch": scratch, "stop": stop}
        msg = handlers[mode](data)
        if msg:
            sys.stderr.write(msg + "\n")
            return 2
        if mode == "pretool":
            out = pretool_warn(data, ctx)
            if out:
                sys.stdout.write(out + "\n")
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
