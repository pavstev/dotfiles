#!/usr/bin/env python3
"""Claude Code guard hook (PreToolUse, UserPromptSubmit, SubagentStop). Python 3 stdlib only.

Usage (wired in settings.json):
  python3 ~/.claude/hooks/guard.py pretool       # matcher Agent|CronCreate|ScheduleWakeup|Skill|AskUserQuestion|SubagentHandback
  python3 ~/.claude/hooks/guard.py prompt        # UserPromptSubmit
  python3 ~/.claude/hooks/guard.py subagentstop  # SubagentStop
  python3 ~/.claude/hooks/guard.py scratch       # agent frontmatter hook, PreToolUse Write|Edit|NotebookEdit
  python3 ~/.claude/hooks/guard.py stop          # Stop

Guards: G1 Agent needs model+effort+deadline+hand-back cap, G2 no polling at big context,
G3 context ceiling (nudge at 300K, deny Agent at 500K), G4 one AskUserQuestion per turn,
G5 subagent hand-back over its cap, G6 a hand-off written this turn must reach the user (Stop:
the reply holds the file link and the paste block; PreToolUse archive_session: not in the same
turn). Mode scratch: writes only under temp or scratchpad folders.
Warnings (allow, add a note): W1 a general-purpose spawn, W2 a final review off opus or a
per-task review on opus.

Deny = message on stderr + exit 2. Allow = exit 0 silently.
Warn = exit 0 with hookSpecificOutput.additionalContext JSON on stdout.
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

MSG_AGENT = (
    'guard: Agent call needs "model" and "effort" (sonnet + high for build/review/'
    "research; opus or fable for plan and final review)."
)
MSG_MAX = (
    'guard: effort "max" is only for plan / synthesis / architecture / final review; '
    'use "high", or say which of those this is in the description.'
)
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
MSG_GENERAL = (
    "guard (warn): a {kind} subagent starts with about 60K tokens of context; `worker` starts "
    "near 15-20K. Next time use `worker` for an edit or command job, `Explore` for a read-only "
    "search, or the named agent for the job (task-implementer, task-reviewer, code-reviewer, "
    "researcher, repo-scanner, ui-verifier). This spawn is allowed."
)
HEAVY_AGENTS = {"general-purpose", "claude"}  # no tools: list, so they load every tool and skill
MSG_FINAL_MODEL = (
    "guard (warn): this looks like a final or whole-branch review on {model}. The owner's rule: "
    "final reviews run on opus (effort max); a cheap judge gives false findings and extra fix rounds."
)
MSG_TASK_MODEL = (
    "guard (warn): a per-task review on {model}. The owner's rule: per-task reviews stay on sonnet; "
    "opus is for the final and whole-branch review only."
)
REVIEW_AGENTS = {"code-reviewer", "task-reviewer"}
STRONG_MODELS = ("opus", "fable")
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
    if fp:
        return MSG_HANDOFF_ARCHIVE.format(path=fp, name=os.path.basename(fp))


def stop(data):
    """G6 at Stop: a hand-off written this turn must be in the final reply. One deny only."""
    if data.get("stop_hook_active") or not data.get("transcript_path"):
        return
    fp, text = handoff_this_turn(data["transcript_path"])
    last = data.get("last_assistant_message")
    if isinstance(last, str) and last not in text:
        text = text + "\n" + last
    if fp and not handoff_given(fp, text):
        return MSG_HANDOFF_REPLY.format(path=fp, name=os.path.basename(fp))


def subagentstop(data):
    if data.get("stop_hook_active"):
        return
    return check_handback(data, data.get("last_assistant_message"))


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


def w1_general(data, ctx):
    """W1: warn on a heavy general-purpose spawn; the owner chose warn, never deny."""
    if data.get("tool_name") != "Agent":
        return
    ti = data.get("tool_input") or {}
    kind = ti.get("subagent_type") or "general-purpose"
    if kind in HEAVY_AGENTS:
        return MSG_GENERAL.format(kind=kind)


def w2_review_model(data, ctx):
    """W2: final reviews on opus, per-task reviews on sonnet."""
    if data.get("tool_name") != "Agent":
        return
    ti = data.get("tool_input") or {}
    kind = ti.get("subagent_type") or ""
    model = str(ti.get("model") or "")
    if not model:
        return  # G1 denies first
    desc = str(ti.get("description") or "")
    head = str(ti.get("prompt") or "")[:400]
    final = bool(_FINAL_REVIEW.search(desc)) or (kind in REVIEW_AGENTS and bool(_FINAL_REVIEW.search(head)))
    strong = any(m in model.lower() for m in STRONG_MODELS)
    if final and not strong:
        return MSG_FINAL_MODEL.format(model=model)
    if kind == "task-reviewer" and strong and not final:
        return MSG_TASK_MODEL.format(model=model)


def pretool(data):
    ctx = Ctx(data)
    for guard in (g3_pretool, g1_agent, g2_polling, g4_ask, g5_handback, g6_archive):
        msg = guard(data, ctx)
        if msg:
            return msg


WARNERS = (w1_general, w2_review_model)


def pretool_warn(data):
    """Warnings for an allowed call: a JSON line for stdout (additionalContext), or None."""
    ctx = Ctx(data)
    notes = []
    for warner in WARNERS:
        try:
            msg = warner(data, ctx)
        except Exception:
            msg = None
        if msg:
            notes.append(msg)
    if notes:
        return json.dumps(
            {"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": "\n".join(notes)}}
        )


def prompt(data):
    """G3 nudge: print JSON for stdout (or None); counter state lives in TMPDIR."""
    ctx = Ctx(data)
    sid = re.sub(r"[^A-Za-z0-9_.-]", "_", str(data.get("session_id") or "unknown"))
    state_dir = _state_dir()
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
    if mode not in ("pretool", "prompt", "subagentstop", "scratch", "stop"):
        return 0
    data = json.loads(sys.stdin.read())
    if not isinstance(data, dict):
        return 0
    if mode in ("pretool", "subagentstop", "scratch", "stop"):
        msg = {"pretool": pretool, "subagentstop": subagentstop, "scratch": scratch, "stop": stop}[mode](data)
        if msg:
            sys.stderr.write(msg + "\n")
            return 2
        if mode == "pretool":
            out = pretool_warn(data)
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
