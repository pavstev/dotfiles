#!/usr/bin/env python3
"""whatsleft-scan: collect proof of what is still open in a Claude Code session.

Reads the session transcript (JSONL) and the git state of every repo the
session touched. Prints tagged evidence lines so the agent can build the
"me / you" table. Read-only, except `git fetch` (skip with --no-fetch).

Tags:
  ASK    a user request — the agent must check it is done, with proof
  ME     open work Claude can do
  YOU    needs the user (a decision, an approval, a login)
  CHECK  maybe open — the agent decides
  LATER  optional, parked in the leftovers file
  OK     nothing to do
  TASK   a running background task or subagent, with a STOP or KEEP advice
  SPEED  a slowness finding from hurry-scan.py (only the worst 3)

Usage: python3 whatsleft-scan.py [--session ID|PATH] [--no-fetch] [--no-ci] [--asks N]
       python3 whatsleft-scan.py --eta     print only: ETA ~35 min (25-50)   (ps:bag-mode uses this)
Stdlib only. Last line: VERDICT: ...
"""
import argparse
import contextlib
import glob
import io
import json
import math
import os
import re
import subprocess
import sys
import time
from datetime import datetime

HOME = os.path.expanduser("~")
PROJECTS = os.path.join(HOME, ".claude", "projects")
LEFTOVERS = os.environ.get("MIC_LEFTOVERS", os.path.join(HOME, ".claude", "leftovers.md"))

CHECK_CMD = re.compile(  # matched with heredoc bodies removed: they are text, not commands
    r"\b(pytest|nextest|vitest|jest|clippy|ruff|biome|golangci-lint|tsc|mypy|shellcheck|pre-commit"
    r"|ty check|go (?:test|vet|build)|cargo (?:test|check|build|clippy)|hook run"
    r"|(?:npm|pnpm|yarn|bun) (?:run )?(?:test|lint|check|typecheck|build)"
    r"|(?:make|just|mise run) (?:test|lint|check|verify))\b"
)
OPEN_WORDS = re.compile(  # phrases that promise or admit open work, not bare words like "failed"
    r"(?i)\bTODO:|\bFIXME\b|\bfollow[- ]?ups?\b|\bnext steps?\b|\bnot (?:\w+ )?yet\b|\bskipped\b"
    r"|\bstill (?:need|needs|have|has) to\b|\byou(?:'ll| will)? (?:need|have|must|should) to\b|\bleft to do\b"
    r"|\b(?:I|we) (?:could not|couldn't|can't|cannot|did not|didn't)\b|\bblocked (?:by|on)\b|\bwaiting (?:for|on)\b"
    r"|\blater\b(?=\s*[.:!]|\s*$)|\b(?:is|are|was|were|still) (?:failing|broken|pending|missing)\b"
)
MARKER = re.compile(  # a marker after a comment sign, or the marker word plus a colon; never the bare word
    r"(?:#|//|/\*|<!--|--|;|^\s*\*)\s*(?:TODO|FIXME|XXX|HACK)\b|\b(?:TODO|FIXME|XXX|HACK)\b(?:\([^)]*\))?:"
)
DENIED = re.compile(r"doesn't want to proceed|was denied|permission .{0,40}denied|rejected by the user", re.I)
BG_RESULT = re.compile(  # only the harness launch line at the START of a result, never quoted output
    r"\s*(?:Command running in background with ID: (\w+)"
    r"|Command did not complete[^\n]{0,80}moved to the background \(ID: (\w+)\))"
)
NOTICE = re.compile(
    r"<task-id>(?P<task>[^<]*)</task-id>\s*<tool-use-id>(?P<tool>[^<]*)</tool-use-id>.*?"
    r"<status>(?P<status>[^<]*)</status>(?:\s*<summary>(?P<summary>[^<]*)</summary>)?",
    re.S,
)
HEREDOC = re.compile(r"<<-?\s*['\"]?(\w+)['\"]?[^\n]*\n.*?\n\s*\1[ \t]*(?=\n|$)", re.S)
WRITE_TOOLS = {"Edit", "Write", "MultiEdit", "NotebookEdit"}
OUTFILE = re.compile(r"(?:Output is being written to:|output_file:)\s*(\S+?\.output)")
AGENT_ID = re.compile(r"agentId:\s*(\w+)")
ANY_ID = re.compile(r"(?i)\b(?:task[_ ]?id|id)[:=\s]+([A-Za-z0-9_-]{6,})")
DEADLINE = re.compile(r"(?i)(?:deadline|done in|within)[^\d\n]{0,20}(\d+)\s*(h(?:ours?|rs?)?|m(?:in(?:ute)?s?)?)\b")
ENDLESS = re.compile(r"next dev|vite|webpack|wrangler dev|(?:npm|pnpm|yarn|bun) (?:run )?dev\b|--watch|tail -f|http\.server|\bserve\b")
BIG_AGENT = re.compile(r"(?i)research|refactor|thorough|migrate|audit")
ARTIFACT_URL = re.compile(r"https://claude\.ai/(?:code/)?artifact/[\w-]+")

counts = {"ME": 0, "YOU": 0, "CHECK": 0, "LATER": 0}
ROWS = []  # (tag, text) of every counted line, for the ETA


def line(tag, text):
    if tag in counts:
        counts[tag] += 1
        ROWS.append((tag, text))
    print(f"  {tag:<6}{text}")


def short(s, n=150):
    s = re.sub(r"\s+", " ", str(s or "")).strip()
    return s if len(s) <= n else s[: n - 1] + "…"


def tilde(p):
    return p.replace(HOME, "~", 1) if p.startswith(HOME) else p


def parse_ts(s):
    try:
        return datetime.fromisoformat(str(s).replace("Z", "+00:00")).timestamp()
    except Exception:
        return None


def ago(ts):
    if not ts:
        return "?"
    m = int((time.time() - ts) / 60)
    return f"{m}m" if m < 120 else f"{m // 60}h{m % 60:02d}m"


def run(args, cwd=None, timeout=20):
    try:
        p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                           env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
        return p.returncode, p.stdout.rstrip()
    except Exception:
        return 1, ""


# --------------------------------------------------------------------------
# 1. Find and read the session
# --------------------------------------------------------------------------
def find_session(arg):
    if arg:
        if os.path.isfile(arg):
            return arg
        hits = glob.glob(os.path.join(PROJECTS, "*", f"{arg}*.jsonl"))
        return hits[0] if hits else None
    sid = os.environ.get("CLAUDE_CODE_SESSION_ID")
    if sid:
        hits = glob.glob(os.path.join(PROJECTS, "*", f"{sid}.jsonl"))
        if hits:
            return hits[0]
    enc = re.sub(r"[^A-Za-z0-9]", "-", os.getcwd())
    mine = glob.glob(os.path.join(PROJECTS, enc, "*.jsonl"))
    return max(mine, key=os.path.getmtime) if mine else None


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text")
    return ""


def clean_ask(t):
    t = re.sub(r"<system-reminder>.*?</system-reminder>", " ", t, flags=re.S)
    t = re.sub(r"<pasted_content[^>]*>.*?</pasted_content>", " [pasted text] ", t, flags=re.S)
    m = re.search(r"<command-name>([^<]*)</command-name>", t)
    if m:
        args = re.search(r"<command-args>([^<]*)</command-args>", t)
        return f"{m.group(1)} {args.group(1) if args else ''}".strip()
    if re.match(r"\s*(<(task-notification|local-command|bash-|command-message)|\[Request interrupted)", t):
        return ""
    return short(t)


def is_human(obj, content):
    if obj.get("isMeta") or obj.get("isSidechain"):
        return False
    origin = obj.get("origin")
    if isinstance(origin, dict) and origin.get("kind") not in (None, "human"):
        return False
    if isinstance(content, list) and any(isinstance(b, dict) and b.get("type") == "tool_result" for b in content):
        return False
    return bool(text_of(content).strip())


def strings(o):
    """Every string inside a JSON value (task notices hide in different fields)."""
    if isinstance(o, str):
        yield o
    elif isinstance(o, dict):
        for v in o.values():
            yield from strings(v)
    elif isinstance(o, list):
        for v in o:
            yield from strings(v)


def read_session(path):
    s = {"asks": [], "uses": {}, "results": {}, "texts": [], "notices": {}, "cwds": set(),
         "order": [], "start": None, "end": None, "todos": None, "tasks": {}, "stopped": set(), "crons": {}}
    with open(path, errors="replace") as f:
        for raw in f:
            try:
                obj = json.loads(raw)
            except Exception:
                continue
            if "<task-notification>" in raw:
                for txt in strings(obj):
                    for n in NOTICE.finditer(txt):
                        s["notices"][n.group("tool")] = (n.group("status"), n.group("summary") or "")
            ts = parse_ts(obj.get("timestamp"))
            if ts:
                s["start"] = s["start"] or ts
                s["end"] = ts
            if obj.get("cwd") and not obj.get("isSidechain"):
                s["cwds"].add(obj["cwd"])
            msg = obj.get("message") or {}
            content = msg.get("content")
            if obj.get("type") == "user" and not obj.get("isSidechain"):
                if is_human(obj, content):
                    ask = clean_ask(text_of(content))
                    if ask:
                        s["asks"].append((ts, ask, len(s["order"])))
                if isinstance(content, list):
                    for b in content:
                        if isinstance(b, dict) and b.get("type") == "tool_result":
                            body = b.get("content")
                            body = body if isinstance(body, str) else text_of(body) or json.dumps(body)[:2000]
                            s["results"][b.get("tool_use_id")] = (bool(b.get("is_error")), body[:2000])
            elif obj.get("type") == "assistant" and not obj.get("isSidechain") and isinstance(content, list):
                for b in content:
                    if not isinstance(b, dict):
                        continue
                    if b.get("type") == "text" and b.get("text", "").strip():
                        s["texts"].append(b["text"])
                    elif b.get("type") == "tool_use":
                        inp = b.get("input") or {}
                        s["uses"][b.get("id")] = (b.get("name"), inp, ts, obj.get("cwd"), len(s["order"]))
                        s["order"].append(b.get("id"))
                        name = b.get("name")
                        if name == "TodoWrite" and isinstance(inp.get("todos"), list):
                            s["todos"] = inp["todos"]
                        elif name == "TaskStop":
                            s["stopped"].add(str(inp.get("task_id") or inp.get("shell_id") or ""))
                        elif name == "CronDelete":
                            s["crons"].pop(str(inp.get("id") or inp.get("job_id") or ""), None)
    # tasks need results, so link them after the read
    for uid in s["order"]:
        name, inp, *_ = s["uses"][uid]
        res = s["results"].get(uid, (False, ""))[1]
        if name == "TaskCreate":
            m = re.search(r"#?(\d+)", res)
            key = m.group(1) if m else uid
            s["tasks"][key] = [inp.get("subject") or inp.get("title") or inp.get("description") or "?", "pending"]
        elif name == "TaskUpdate":
            key = str(inp.get("taskId") or inp.get("task_id") or inp.get("id") or "")
            if key in s["tasks"] and inp.get("status"):
                s["tasks"][key][1] = inp["status"]
        elif name == "CronCreate":
            m = re.search(r"\b(?:id|ID|job)[:\s]+([\w-]+)", res)
            s["crons"][m.group(1) if m else uid] = inp.get("prompt") or inp.get("cron") or "?"
    return s


def describe(name, inp):
    d = inp.get("description") or inp.get("command") or inp.get("prompt") or inp.get("file_path") or ""
    return f'{name} "{short(d, 80)}"'


# --------------------------------------------------------------------------
# 2. Session sections
# --------------------------------------------------------------------------
def section_asks(s, limit):
    asks = s["asks"]
    print(f"== ASKS ({len(asks)}) — check each one: done, with proof from this session? ==")
    if len(asks) > limit:
        print(f"  …     {len(asks) - limit} older ask(s) not shown (--asks N)")
    for i, (ts, ask, _) in enumerate(asks[-limit:], start=max(1, len(asks) - limit + 1)):
        when = datetime.fromtimestamp(ts).strftime("%H:%M") if ts else "--:--"
        print(f"  ASK   #{i} {when}  {ask}")
    if not asks:
        print("  ASK   none found — ask the user what they wanted")
    print()


def section_todos(s):
    items = []
    if s["todos"]:
        items += [(t.get("content") or t.get("activeForm") or "?", t.get("status", "?")) for t in s["todos"]]
    items += [(subj, st) for subj, st in s["tasks"].values()]
    if not items:
        return
    print("== TODO LIST ==")
    open_ = [(c, st) for c, st in items if st not in ("completed", "deleted", "cancelled")]
    for c, st in open_:
        line("ME", f"{st}: {short(c, 120)}")
    if not open_:
        line("OK", f"all {len(items)} todo item(s) completed")
    print()


def background_tasks(s):
    """Every background task or subagent this session started, with its state."""
    rows = []
    for uid in s["order"]:
        name, inp, ts, _, _ = s["uses"][uid]
        is_err, res = s["results"].get(uid, (False, ""))
        m = BG_RESULT.match(res)
        task_id = (m.group(1) or m.group(2)) if m else ""
        is_bg = bool(m) or res.lstrip().startswith("Async agent launched") or (
            inp.get("run_in_background") is True and name in ("Bash", "Agent", "Monitor"))
        if not is_bg:
            continue
        if not task_id:
            a = AGENT_ID.search(res) or ANY_ID.search(res)
            task_id = a.group(1) if a else uid
        status, summary = s["notices"].get(uid, ("", ""))
        if task_id in s["stopped"]:
            status = status or "stopped"
        o = OUTFILE.search(res)
        d = DEADLINE.search(inp.get("prompt") or "")
        rows.append({"id": task_id, "name": name, "inp": inp, "ts": ts, "status": status, "summary": summary,
                     "out": o.group(1) if o else "", "deadline": int(d.group(1)) * (60 if d.group(2)[0] in "hH" else 1) if d else 0})
    return rows


def advise(t):
    """STOP or KEEP for one running task, with the reason. Sets t['stop'], t['out_age'], t['run_min']."""
    now = time.time()
    t["run_min"] = (now - t["ts"]) / 60 if t["ts"] else 0
    mt = os.path.getmtime(t["out"]) if t["out"] and os.path.exists(t["out"]) else None
    t["out_age"] = (now - mt) / 60 if mt else None
    cmd = t["inp"].get("command") or ""
    t["endless"] = bool(ENDLESS.search(cmd))
    run, quiet = t["run_min"], t["out_age"]
    if t["out"] and mt is None and run > 5:
        return True, "output file is gone, the task is dead"
    if t["deadline"] and run > 1.5 * t["deadline"]:
        return True, f"past its {t['deadline']}m deadline"
    if t["endless"]:
        if quiet is not None and quiet >= 30:
            return False, f"idle server ({int(quiet)}m quiet), stop it when work ends"
        return False, "server, stop it when work ends"
    if quiet is not None and quiet >= 20 and run >= 20:
        return False, f"quiet for {int(quiet)}m, read its output before stopping"
    if run < 5:
        return False, "just started"
    return False, "output is recent" if quiet is not None and quiet < 20 else "inside its time"


def section_background(s):
    rows = background_tasks(s)
    if not rows and not s["crons"]:
        return
    print("== BACKGROUND WORK ==")
    running = [t for t in rows if not t["status"]]
    stops = 0
    for t in running:
        stop, why = advise(t)
        t["stop"] = stop
        stops += stop
        kind = {"Agent": "agent", "Bash": "shell", "Monitor": "monitor"}.get(t["name"], t["name"].lower())
        what = short(t["inp"].get("description") or t["inp"].get("command") or t["inp"].get("prompt"), 50)
        last = "no file" if t["out_age"] is None else f"{int(t['out_age'])}m ago"
        print(f"  TASK  {t['id']} | {kind} | {what} | {int(t['run_min'])}m | {last} | {'STOP' if stop else 'KEEP'}: {why}")
    if stops:
        line("ME", f"stop {stops} dead task(s) marked STOP above (TaskStop)")
    for t in rows:
        if t["status"] == "failed":
            line("CHECK", f"failed: {describe(t['name'], t['inp'])} — {short(t['summary'], 90)} — still needed?")
    for what in s["crons"].values():
        line("CHECK", f"scheduled job still set: {short(what, 90)} — keep it?")
    done = sum(1 for t in rows if t["status"] in ("completed", "killed", "stopped"))
    if done:
        line("OK", f"{done} background task(s) finished or stopped")
    print()


def section_speed(path):
    """The worst slowness findings, from hurry-scan.py (same session)."""
    script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "hurry-scan.py")
    rc, out = run([sys.executable, script, "--brief", "--session", path], timeout=30)
    if rc == 0 and out.strip():
        print("== SPEED (slow session? [ME] rows are mine, [YOU] rows are yours) ==")
        print(out)
        print()


def section_deliverable(s, roots):
    """Best guess for the first line of the reply: artifact URL, then commit sha, then last written file."""
    urls = ARTIFACT_URL.findall("\n".join(r[1] for r in s["results"].values()))
    if urls:
        what = urls[-1]
    else:
        what = ""
        for r in roots:
            since = datetime.fromtimestamp(s["start"]).isoformat() if s["start"] else "1 day ago"
            _, sha = run(["git", "-C", r, "log", f"--since={since}", "--format=%h", "-1"], timeout=10)
            if sha:
                what = f"{sha} ({os.path.basename(r)})"
                break
        if not what:
            files = [i.get("file_path") for n, i, *_ in s["uses"].values() if n in ("Write", "NotebookEdit") and i.get("file_path")]
            what = tilde(files[-1]) if files else ""
    print(f"DELIVERABLE  {what or 'none yet'}\n")


def estimate(s):
    """One ETA line: (mid, low, high) in minutes. Uses the counted rows and the running tasks."""
    serial = 0
    for tag, text in ROWS:
        t = text.lower()
        if tag == "YOU" or t.startswith("stop "):
            continue
        if "ci run" in t or "ci failed" in t:
            serial += 10
        elif "check" in t or "lint" in t:
            serial += 2
        elif "pushed" in t or "committed" in t or "pulled" in t:
            serial += 1
        elif "half done" in t or "conflict" in t:
            serial += 5
        elif tag == "CHECK":
            serial += 1
        else:
            serial += 3 if t.split(":")[0] in ("pending", "in_progress") else 2
    parallel = 0  # tasks that run side by side count once: the slowest one
    for t in background_tasks(s):
        if t["status"] or advise(t)[0] or t["endless"]:
            continue
        if t["name"] == "Agent":
            left = t["deadline"] - t["run_min"] if t["deadline"] else (10 if BIG_AGENT.search(t["inp"].get("prompt") or "") else 5)
            parallel = max(parallel, max(left, 2))
        else:
            parallel = max(parallel, 3)
    mid = serial + parallel
    up5 = lambda x: int(math.ceil(x / 5.0) * 5)  # noqa: E731
    return up5(mid), max(5, up5(mid * 0.7)) if mid else 0, up5(mid * 2)


def eta_line(s):
    mid, low, high = estimate(s)
    return "ETA ~0 min (nothing open)" if not mid else f"ETA ~{mid} min ({low}-{high})"


def section_recent(s):
    """Errors and denials after the second-to-last ask: the last work turn."""
    asks = s["asks"]
    start = asks[-2][2] if len(asks) >= 2 else 0
    errs, denials, seen = [], [], set()
    for uid in s["order"][start:]:
        name, inp, *_ = s["uses"][uid]
        is_err, res = s["results"].get(uid, (False, ""))
        if not is_err:
            continue
        key = short(res, 60)
        if key in seen:
            continue
        seen.add(key)
        (denials if DENIED.search(res) else errs).append((describe(name, inp), short(res, 100)))
    if not errs and not denials:
        return
    print("== RECENT ERRORS (last work turn) ==")
    for desc, _ in denials[-5:]:
        line("YOU", f"you said no to: {desc} — do it yourself, or tell me another way")
    for desc, res in errs[-5:]:
        line("CHECK", f"error: {desc} → {res} — fixed later?")
    print()


def section_open_words(s):
    hits = []
    for t in s["texts"][-4:]:
        t = re.sub(r"```.*?```", " ", t, flags=re.S)  # code blocks are examples, not promises
        for ln in t.splitlines():
            if OPEN_WORDS.search(ln) and len(ln.strip()) > 12:
                hits.append(short(ln.strip(" -*#>|"), 140))
    hits = list(dict.fromkeys(hits))[-8:]
    if not hits:
        return
    print("== OPEN WORDS (from Claude's last replies) ==")
    for h in hits:
        line("CHECK", f'"{h}"')
    print()


# --------------------------------------------------------------------------
# 3. Repos the session touched
# --------------------------------------------------------------------------
def touched_repos(s):
    dirs = set(s["cwds"]) | {os.getcwd()}   # the folder we run in always counts, even in a subagent
    for name, inp, *_ in s["uses"].values():
        p = inp.get("file_path") or inp.get("notebook_path")
        if name in WRITE_TOOLS and p:
            dirs.add(os.path.dirname(p))
        cmd = inp.get("command") or ""
        for m in re.finditer(r"(?:git -C|cd)\s+(\"[^\"]+\"|'[^']+'|[^\s;&|]+)", cmd):
            dirs.add(os.path.expanduser(m.group(1).strip("'\"")))
    roots = {}
    for d in dirs:
        while d and not os.path.isdir(d):
            d = os.path.dirname(d)
        if not d:
            continue
        rc, top = run(["git", "-C", d, "rev-parse", "--show-toplevel"], timeout=5)
        if rc == 0 and top:
            roots[top] = True
    return sorted(roots)


def in_repo(path, root):
    return bool(path) and (path == root or path.startswith(root + os.sep))


def check_freshness(s, root):
    """ME line when files were edited after the last check that ran in this repo."""
    last_edit = last_check = None
    check_failed = False
    for uid in s["order"]:
        name, inp, _, cwd, idx = s["uses"][uid]
        if name in WRITE_TOOLS and in_repo(inp.get("file_path") or inp.get("notebook_path"), root):
            last_edit = idx
        cmd = inp.get("command") or ""
        bare = HEREDOC.sub(" ", cmd)
        if name == "Bash" and CHECK_CMD.search(bare) and (in_repo(cwd, root) or root in os.path.expanduser(bare)):
            last_check = idx
            check_failed = s["results"].get(uid, (False, ""))[0]
    if last_edit is None:
        return
    if last_check is None:
        line("ME", "files edited, but no test/lint/check ran in this repo this session")
    elif last_check < last_edit:
        line("ME", "files edited after the last check run — run the checks again")
    elif check_failed:
        line("ME", "the last check run failed — fix it, then run it again")
    else:
        line("OK", "a check ran after the last edit, and it passed")


def new_markers(root, since):
    out = []
    cmds = [["git", "diff", "HEAD", "-U0"]]
    if since:  # commits made during this session
        cmds.append(["git", "log", f"--since={datetime.fromtimestamp(since).isoformat()}", "-p", "-U0", "--format="])
    for args in cmds:
        _, diff = run(args, cwd=root)
        f = "?"
        for ln in diff.splitlines():
            if ln.startswith("+++ "):
                f = ln[6:] if ln.startswith("+++ b/") else ln[4:]
            elif ln.startswith("+") and MARKER.search(ln):
                out.append(f"{f}: {short(ln[1:].strip(), 90)}")
    _, untracked = run(["git", "ls-files", "--others", "--exclude-standard"], cwd=root)
    for f in untracked.splitlines()[:50]:
        try:
            with open(os.path.join(root, f), errors="replace") as fh:
                for ln in fh.read(200_000).splitlines():
                    if MARKER.search(ln):
                        out.append(f"{f}: {short(ln.strip(), 90)}")
        except OSError:
            pass
    return list(dict.fromkeys(out))


def section_repo(s, root, fetch, ci):
    g = lambda *a, t=20: run(["git", "-C", root, *a], timeout=t)  # noqa: E731
    print(f"== REPO {os.path.basename(root)}  ({tilde(root)}) ==")
    if fetch:
        rc, _ = g("fetch", "--quiet", "--prune", t=25)
        if rc != 0:
            line("CHECK", "git fetch failed — remote state may be old")
    _, gd = g("rev-parse", "--absolute-git-dir")
    _, branch = g("symbolic-ref", "--quiet", "--short", "HEAD")
    _, main = g("symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD")
    main = main.replace("origin/", "", 1) or "main"

    for marker, what in (("MERGE_HEAD", "merge"), ("rebase-merge", "rebase"), ("rebase-apply", "rebase"),
                         ("CHERRY_PICK_HEAD", "cherry-pick"), ("REVERT_HEAD", "revert")):
        if gd and os.path.exists(os.path.join(gd, marker)):
            line("ME", f"a {what} is half done — finish it")
            break
    _, conflicts = g("diff", "--name-only", "--diff-filter=U")
    if conflicts:
        line("ME", f"merge conflicts in: {short(' '.join(conflicts.splitlines()), 100)}")

    if not branch:
        line("ME", "detached HEAD — put the work on a branch")
    elif branch != main:
        line("YOU", f"on branch '{branch}', not '{main}' — merge it, or open a PR? (your call)")

    _, status = g("status", "--porcelain")
    if status:
        st = status.splitlines()
        new = sum(1 for x in st if x.startswith("??"))
        line("ME", f"not committed: {len(st) - new} changed, {new} new file(s)")
        for x in st[:6]:
            print(f"          {x}")
        if len(st) > 6:
            print(f"          … {len(st) - 6} more")
    else:
        line("OK", "working tree clean")

    if branch:
        rc, up = g("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}")
        if rc != 0 or not up:
            line("ME", f"branch '{branch}' was never pushed (no upstream)")
        else:
            _, lr = g("rev-list", "--left-right", "--count", f"{up}...HEAD")
            behind, ahead = (int(x) for x in (lr.split() or ["0", "0"]))
            if ahead:
                line("ME", f"{ahead} commit(s) not pushed to {up}")
            if behind:
                line("ME", f"{behind} commit(s) on {up} not pulled")
            if not ahead and not behind:
                _, sha = g("rev-parse", "--short", "HEAD")
                line("OK", f"in sync with {up} ({sha})")

    _, stash = g("stash", "list")
    if stash:
        line("CHECK", f"{len(stash.splitlines())} stash(es) — forgotten work? ({short(stash.splitlines()[0], 70)})")

    check_freshness(s, root)

    if ci and branch and glob.glob(os.path.join(root, ".github", "workflows", "*.y*ml")):
        _, head = g("rev-parse", "HEAD")
        rc, runs = run(["gh", "run", "list", "--commit", head, "--json", "name,status,conclusion",
                        "-q", '.[] | "\\(.status)|\\(.conclusion)|\\(.name)"'], cwd=root, timeout=20)
        if rc != 0:
            line("CHECK", "could not read CI (gh missing or not logged in)")
        elif runs:
            rows = [r.split("|", 2) for r in runs.splitlines()]
            bad = sorted({r[2] for r in rows if r[0] == "completed" and r[1] in
                          ("failure", "cancelled", "timed_out", "startup_failure")})
            pending = sum(1 for r in rows if r[0] != "completed")
            if bad:
                line("ME", f"CI failed on HEAD: {', '.join(bad)}")
            if pending:
                line("CHECK", f"{pending} CI run(s) still running on HEAD — wait, then look again")
            if not bad and not pending:
                line("OK", "CI green on HEAD")

    for m in new_markers(root, s["start"])[:5]:
        line("CHECK", f"new TODO marker: {m}")
    print()


def section_leftovers(roots):
    try:
        with open(LEFTOVERS) as f:
            rows = [ln.strip() for ln in f if ln.startswith("- [ ]")]
    except OSError:
        return
    names = {os.path.basename(r) for r in roots}
    mine = [r for r in rows if any(f"**{n}**" in r for n in names)]
    if not mine:
        return
    print(f"== LEFTOVERS ({tilde(LEFTOVERS)}) ==")
    for r in mine[-8:]:
        line("LATER", short(r[6:], 130))
    print()


# --------------------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--session", help="session id (prefix) or .jsonl path; default: this session")
    ap.add_argument("--no-fetch", action="store_true", help="skip git fetch")
    ap.add_argument("--no-ci", action="store_true", help="skip the GitHub CI lookup")
    ap.add_argument("--asks", type=int, default=25, help="max asks to print (default 25)")
    ap.add_argument("--eta", action="store_true", help="print only the ETA line (implies --no-fetch)")
    a = ap.parse_args()

    path = find_session(a.session)
    if not path:
        print("No session transcript found. Pass --session <id|path>.")
        print("VERDICT: NO SESSION — build the table from this conversation only")
        return
    s = read_session(path)
    if a.eta:
        a.no_fetch = True
        with contextlib.redirect_stdout(io.StringIO()):
            for r in touched_repos(s):
                section_repo(s, r, False, not a.no_ci)
            section_todos(s)
            section_background(s)
        print(eta_line(s))
        return
    dur = int(((s["end"] or 0) - (s["start"] or 0)) / 60)
    sid = os.path.basename(path)[:8]
    print(f"WHATSLEFT  session {sid} · {dur // 60}h{dur % 60:02d}m · "
          f"{len(s['asks'])} asks · {len(s['order'])} tool calls\n")

    roots = touched_repos(s)
    section_deliverable(s, roots)
    section_asks(s, a.asks)
    section_todos(s)
    section_background(s)
    section_recent(s)
    section_open_words(s)
    section_speed(path)
    for r in roots:
        section_repo(s, r, not a.no_fetch, not a.no_ci)
    if not roots:
        print("== REPOS ==\n  OK    no git repo touched\n")
    section_leftovers(roots)

    print(eta_line(s))
    total = counts["ME"] + counts["YOU"] + counts["CHECK"]
    later = f" · LATER {counts['LATER']}" if counts["LATER"] else ""
    head = f"{total} open" if total else "NOTHING OPEN FOUND"
    print(f"VERDICT: {head} — ME {counts['ME']} · YOU {counts['YOU']} · CHECK {counts['CHECK']}{later} "
          f"(+ {len(s['asks'])} asks to cross-check)")


if __name__ == "__main__":
    main()
