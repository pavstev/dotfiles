#!/usr/bin/env python3
"""Cursor port of guard G3 (context ceiling). Python 3 stdlib only.

Usage (wired in ~/.cursor/hooks.json, event JSON on stdin):
  python3 <repo>/cursor/hooks/guard_cursor.py prompt     # beforeSubmitPrompt
  python3 <repo>/cursor/hooks/guard_cursor.py subagent   # subagentStart, preToolUse (Task)
  python3 <repo>/cursor/hooks/guard_cursor.py compact    # preCompact
  python3 <repo>/cursor/hooks/guard_cursor.py response   # afterAgentResponse

Limits and the transcript reader come from claude/hooks/guard.py (imported read-only).
Cursor hooks carry no context size, so this keeps its own per-conversation record.
Only an explicit context size counts: `context_tokens` in afterAgentResponse (top level
or inside a usage object). Summed input tokens are ignored, because Cursor may report
them per turn, and a turn total over 500K would deny spawns by mistake. preCompact
clears the record, as guard.py does: the size before compaction is stale after it.

Context estimate, first hit wins:
1. guard.context_size(transcript_path) when it returns > 0 (read at hook time).
2. Cursor.app's own count: `contextTokensUsed` in the app's chat store
   (globalStorage/state.vscdb, key `composerData:<conversation_id>`), read-only.
   The app sends no transcript and only per-turn token totals, so this is the exact
   size there (live 2026-10-13: store 32,539 vs hook input_tokens 138,002).
   CURSOR_GUARD_APP_DB overrides the path (tests); the CLI store is encrypted.
3. The latest recorded value.
With none, the guard stays silent (best effort; docs/cursor-port.md, "Gaps").

Nudge (prompt) = {"continue": true, "user_message": MSG, "additional_context": ...}.
The app does not show user_message here, so additional_context asks the agent to say it.
Deny (subagent) = {"permission": "deny", "user_message": MSG} on stdout, MSG on
stderr, exit 2. A prompt is never blocked.
Fail-open: any exception, bad input or unknown mode exits 0 with no output.
"""
import json
import os
import re
import sqlite3
import sys
import time
from urllib.request import pathname2url

HERE = os.path.dirname(os.path.realpath(__file__))
CLAUDE_HOOKS = os.path.normpath(os.path.join(HERE, "..", "..", "claude", "hooks"))
STATE_SUBDIR = "cursor-guard"
MAX_STATE_FILES = 200  # keep the newest this many conversation records
MAX_STATE_AGE = 7 * 24 * 3600  # drop records older than a week
MODES = ("prompt", "subagent", "compact", "response")
APP_DB = os.path.expanduser("~/Library/Application Support/Cursor/User/globalStorage/state.vscdb")
SUBAGENT_TOOLS = ("Task", "Subagent", "mcp_task")  # preToolUse names for a subagent spawn

# Token field names in afterAgentResponse are UNCONFIRMED; these are tried in order.
DIRECT_KEYS = ("context_tokens", "contextTokens")
USAGE_KEYS = ("usage", "token_usage", "tokenUsage", "tokens")


def load_guard():
    """Import claude/hooks/guard.py from this repo, or None."""
    sys.dont_write_bytecode = True  # no __pycache__ inside claude/hooks (linked into ~/.claude)
    if CLAUDE_HOOKS not in sys.path:
        sys.path.insert(0, CLAUDE_HOOKS)
    try:
        import guard
    except Exception:
        return None
    return guard


# --- state ---------------------------------------------------------------------


def conv_key(data, mode):
    raw = data.get("conversation_id")
    if mode == "subagent" and data.get("parent_conversation_id"):
        raw = data.get("parent_conversation_id")
    key = re.sub(r"[^A-Za-z0-9_.-]", "_", str(raw or "unknown"))[:128]
    return key if key.strip(".") else "unknown"


def state_dir():
    return os.path.join(os.environ.get("TMPDIR") or "/tmp", STATE_SUBDIR)


def read_state(key):
    try:
        with open(os.path.join(state_dir(), key + ".json")) as f:
            st = json.load(f)
        return st if isinstance(st, dict) else {}
    except Exception:
        return {}


def prune(d):
    """Keep the state dir small: drop old records, cap the count."""
    try:
        now = time.time()
        files = []
        for name in os.listdir(d):
            if not name.endswith(".json"):
                continue
            p = os.path.join(d, name)
            mt = os.path.getmtime(p)
            if now - mt > MAX_STATE_AGE:
                os.remove(p)
            else:
                files.append((mt, p))
        files.sort(reverse=True)
        for _, p in files[MAX_STATE_FILES:]:
            os.remove(p)
    except Exception:
        pass


def write_state(key, st):
    try:
        d = state_dir()
        os.makedirs(d, exist_ok=True)
        st["updated"] = int(time.time())
        tmp = os.path.join(d, f".{key}.{os.getpid()}.tmp")
        with open(tmp, "w") as f:
            json.dump(st, f)
        os.replace(tmp, os.path.join(d, key + ".json"))
        prune(d)
    except Exception:
        pass


# --- context size ------------------------------------------------------------


def _int(v):
    if isinstance(v, bool):
        return 0
    if isinstance(v, (int, float)):
        return int(v) if v > 0 else 0
    if isinstance(v, str) and v.strip().isdigit():
        return int(v.strip())
    return 0


def tokens_from(obj):
    """An explicit context size from a usage-like dict; 0 if none found."""
    if not isinstance(obj, dict):
        return 0
    for k in DIRECT_KEYS:
        n = _int(obj.get(k))
        if n:
            return n
    return 0


def response_tokens(data):
    for k in USAGE_KEYS:
        n = tokens_from(data.get(k))
        if n:
            return n
    return tokens_from(data)


def app_context(data):
    """contextTokensUsed from Cursor.app's chat store, or 0."""
    conv = data.get("parent_conversation_id") or data.get("conversation_id")
    path = os.environ.get("CURSOR_GUARD_APP_DB") or APP_DB
    if not isinstance(conv, str) or not conv or not os.path.isfile(path):
        return 0
    try:
        con = sqlite3.connect("file:" + pathname2url(path) + "?mode=ro", uri=True, timeout=1)
        try:
            row = con.execute("SELECT value FROM cursorDiskKV WHERE key = ?", ("composerData:" + conv,)).fetchone()
        finally:
            con.close()
        meta = json.loads(row[0]) if row and row[0] else {}
    except Exception:
        return 0
    if not isinstance(meta, dict):
        return 0
    n = _int(meta.get("contextTokensUsed"))
    if not n and isinstance(meta.get("promptTokenBreakdown"), dict):
        n = _int(meta["promptTokenBreakdown"].get("totalUsedTokens"))
    return n


def estimate(guard, data, st):
    path = data.get("transcript_path")
    if isinstance(path, str) and path and os.path.isfile(path):
        n = guard.context_size(path)
        if n > 0:
            return n
    return app_context(data) or _int(st.get("ctx"))


# --- modes ---------------------------------------------------------------------


def prompt(guard, data, key):
    """Nudge at NUDGE_CONTEXT+: first prompt there, then every NUDGE_EVERY-th."""
    st = read_state(key)
    size = estimate(guard, data, st)
    count = _int(st.get("prompts"))
    count = count + 1 if size >= guard.NUDGE_CONTEXT else 0
    st["prompts"] = count
    write_state(key, st)
    out = {"continue": True}
    if count >= 1 and (count - 1) % guard.NUDGE_EVERY == 0:
        msg = f"Context is about {size // 1000}K. Run /ps:handoff and continue in a fresh session."
        out["user_message"] = msg  # Cursor.app 3.24.12 does not show this with continue: true (live probe)
        # The app does pass this to the agent (live probe), so the agent relays the nudge.
        out["additional_context"] = f"Start your reply by telling the user, in one line: {msg}"
    return out


def subagent(guard, data, key):
    tool = data.get("tool_name")
    if tool is not None and tool not in SUBAGENT_TOOLS:
        return None  # preToolUse for some other tool
    size = estimate(guard, data, read_state(key))
    if size > guard.SPAWN_CONTEXT:
        return (
            f"guard: context is {size // 1000}K (> {guard.SPAWN_CONTEXT // 1000}K); run "
            "/ps:handoff and continue in a fresh session instead of spawning."
        )


def record(key, n):
    if n > 0:
        st = read_state(key)
        st["ctx"] = n
        write_state(key, st)


def clear(key):
    """After compaction the old size is stale (guard.py does the same)."""
    st = read_state(key)
    if st.get("ctx") or st.get("prompts"):
        st["ctx"] = 0
        st["prompts"] = 0
        write_state(key, st)


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode not in MODES:
        return 0
    guard = load_guard()
    if guard is None:
        return 0
    data = json.loads(sys.stdin.read())
    if not isinstance(data, dict):
        return 0
    key = conv_key(data, mode)
    if mode == "prompt":
        sys.stdout.write(json.dumps(prompt(guard, data, key)) + "\n")
    elif mode == "subagent":
        msg = subagent(guard, data, key)
        if msg:
            out = {"permission": "deny", "user_message": msg, "agent_message": msg}
            sys.stdout.write(json.dumps(out) + "\n")
            sys.stderr.write(msg + "\n")
            return 2
    elif mode == "compact":
        clear(key)
    else:
        record(key, response_tokens(data))
    return 0


if __name__ == "__main__":
    try:
        code = main()
    except Exception:
        code = 0
    sys.exit(code)
