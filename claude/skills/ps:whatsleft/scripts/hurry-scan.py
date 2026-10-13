#!/usr/bin/env python3
"""hurry-scan: find out why a Claude Code session is slow, and what to cut.

Reads the session transcript (JSONL) plus the static weight Claude loads on
every turn (CLAUDE.md, skills, agents, MCP, hooks, plugins). Prints a short,
ranked report. Read-only: it never changes anything.

Usage: python3 hurry-scan.py [--session ID|PATH] [--all] [--window N] [--json] [--brief]
--brief prints at most 3 SPEED lines (score 60+) for whatsleft-scan.py and nothing else.
Stdlib only. Works on macOS and Linux.
"""
import argparse
import glob
import json
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict
from datetime import datetime

HOME = os.path.expanduser("~")
CLAUDE = os.path.join(HOME, ".claude")
PROJECTS = os.path.join(CLAUDE, "projects")


def tok(chars):
    """Cheap token estimate: 4 chars per token."""
    return int(chars / 4)


def human(n):
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f}{unit}" if unit == "B" else f"{n:.1f}{unit}"
        n /= 1024


def ktok(n):
    return f"{n / 1000:.0f}k" if n >= 1000 else str(n)


def parse_ts(s):
    try:
        return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()
    except Exception:
        return None


# --------------------------------------------------------------------------
# 1. Find the session transcript
# --------------------------------------------------------------------------
def find_session(arg, use_all):
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
    if not use_all:
        enc = re.sub(r"[^A-Za-z0-9]", "-", os.getcwd())
        mine = glob.glob(os.path.join(PROJECTS, enc, "*.jsonl"))
        if mine:
            return max(mine, key=os.path.getmtime)
    everything = glob.glob(os.path.join(PROJECTS, "*", "*.jsonl"))
    return max(everything, key=os.path.getmtime) if everything else None


# --------------------------------------------------------------------------
# 2. Read the transcript
# --------------------------------------------------------------------------
def result_size(content):
    if isinstance(content, str):
        return len(content)
    if isinstance(content, list):
        return sum(len(b.get("text", "")) if isinstance(b, dict) else len(str(b)) for b in content)
    return len(str(content))


def short_input(name, inp):
    if not isinstance(inp, dict):
        return ""
    for key in ("file_path", "command", "pattern", "url", "query", "description", "prompt"):
        if key in inp:
            return re.sub(r"\s+", " ", str(inp[key]))[:70]
    return ""


def scan_transcript(path):
    s = {
        "path": path, "bytes": os.path.getsize(path), "turns_user": 0, "turns_assistant": 0,
        "sidechain": 0, "model": None, "ctx_now": 0, "ctx_max": 0, "compactions": [],
        "tool_calls": Counter(), "tool_bytes": Counter(), "results": [], "slow": [],
        "reads": Counter(), "think_ms": 0, "errors": 0, "first_ts": None, "last_ts": None,
        "cwd": None, "attach_by_type": Counter(), "mcp_tools": set(), "interrupts": 0,
    }
    pending = {}  # tool_use_id -> (name, summary, ts)
    with open(path, "r", errors="replace") as fh:
        for line in fh:
            try:
                o = json.loads(line)
            except Exception:
                continue
            t = o.get("type")
            ts = parse_ts(o.get("timestamp", "")) if o.get("timestamp") else None
            if ts:
                s["first_ts"] = s["first_ts"] or ts
                s["last_ts"] = ts
            s["cwd"] = s["cwd"] or o.get("cwd")
            if o.get("isSidechain"):
                s["sidechain"] += 1
            if t == "system" and o.get("subtype") == "compact_boundary":
                meta = o.get("compactMetadata") or {}
                s["compactions"].append((meta.get("preTokens"), meta.get("postTokens"), meta.get("durationMs")))
            elif t == "attachment":
                att = o.get("attachment") or {}
                s["attach_by_type"][att.get("type", "?")] += len(line)
                for n in att.get("addedNames") or []:
                    if isinstance(n, str) and n.startswith("mcp__"):
                        s["mcp_tools"].add(n)
            elif t == "assistant":
                msg = o.get("message", {})
                s["turns_assistant"] += 1
                s["think_ms"] += o.get("thinkingDurationMs") or 0
                s["model"] = msg.get("model") or s["model"]
                u = msg.get("usage") or {}
                ctx = (u.get("input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0) + (u.get("cache_creation_input_tokens") or 0)
                if ctx and not o.get("isSidechain"):
                    s["ctx_now"] = ctx
                    s["ctx_max"] = max(s["ctx_max"], ctx)
                for b in msg.get("content", []) or []:
                    if isinstance(b, dict) and b.get("type") == "tool_use":
                        name = b.get("name", "?")
                        inp = b.get("input", {})
                        s["tool_calls"][name] += 1
                        pending[b.get("id")] = (name, short_input(name, inp), ts)
                        if name == "Read" and isinstance(inp, dict) and inp.get("file_path"):
                            s["reads"][inp["file_path"]] += 1
            elif t == "user":
                s["turns_user"] += 1
                content = (o.get("message") or {}).get("content")
                if isinstance(content, list):
                    for b in content:
                        if not isinstance(b, dict):
                            continue
                        if b.get("type") == "tool_result":
                            name, summ, t0 = pending.pop(b.get("tool_use_id"), ("?", "", None))
                            size = result_size(b.get("content"))
                            s["tool_bytes"][name] += size
                            s["results"].append((size, name, summ))
                            if b.get("is_error"):
                                s["errors"] += 1
                            if t0 and ts and ts - t0 > 20:
                                s["slow"].append((ts - t0, name, summ))
                        elif b.get("type") == "text" and "Request interrupted" in b.get("text", ""):
                            s["interrupts"] += 1
    s["results"].sort(reverse=True)
    s["slow"].sort(reverse=True)
    return s


# --------------------------------------------------------------------------
# 3. Static weight: what loads on every turn
# --------------------------------------------------------------------------
def read_text(p):
    try:
        with open(p, "r", errors="replace") as f:
            return f.read()
    except Exception:
        return ""


def load_json(p):
    try:
        with open(p) as f:
            return json.load(f)
    except Exception:
        return {}


def claude_md_files(cwd):
    found = []
    g = os.path.join(CLAUDE, "CLAUDE.md")
    if os.path.exists(g):
        found.append(g)
    d = cwd or os.getcwd()
    seen = set()
    while d and d != "/" and d not in seen:
        seen.add(d)
        for name in ("CLAUDE.md", "AGENTS.md", os.path.join(".claude", "CLAUDE.md")):
            p = os.path.join(d, name)
            if os.path.exists(p) and os.path.realpath(p) not in [os.path.realpath(x) for x in found]:
                found.append(p)
        if os.path.isdir(os.path.join(d, ".git")) or d == HOME:
            break
        d = os.path.dirname(d)
    rules = glob.glob(os.path.join(CLAUDE, "rules", "**", "*.md"), recursive=True)
    return found, rules


def frontmatter_desc(path):
    txt = read_text(path)
    m = re.search(r"^description:\s*(.*?)(?:\n[a-zA-Z_-]+:|\n---)", txt, re.S | re.M)
    return len(m.group(1)) if m else 0


def static_weight(cwd):
    w = {}
    files, rules = claude_md_files(cwd)
    w["claude_md"] = [(p, len(read_text(p))) for p in files]
    w["rules"] = (len(rules), sum(len(read_text(p)) for p in rules))
    skills = [p for p in glob.glob(os.path.join(CLAUDE, "skills", "*", "SKILL.md"))]
    w["skills"] = (len(skills), sum(frontmatter_desc(p) for p in skills))
    plugin_skills = glob.glob(os.path.join(CLAUDE, "plugins", "cache", "**", "SKILL.md"), recursive=True)
    w["plugin_skills"] = (len(plugin_skills), sum(frontmatter_desc(p) for p in plugin_skills))
    agents = glob.glob(os.path.join(CLAUDE, "agents", "*.md"))
    w["agents"] = (len(agents), sum(frontmatter_desc(p) for p in agents))
    mem = glob.glob(os.path.join(CLAUDE, "projects", "*", "memory", "MEMORY.md"))
    w["memory_index"] = sum(len(read_text(p)) for p in mem if cwd and os.path.basename(os.path.dirname(os.path.dirname(p))) == re.sub(r"[^A-Za-z0-9]", "-", cwd))
    settings = load_json(os.path.join(CLAUDE, "settings.json"))
    local = load_json(os.path.join(cwd or os.getcwd(), ".claude", "settings.json"))
    hooks = {}
    for src in (settings, local):
        for ev, groups in (src.get("hooks") or {}).items():
            n = 0
            for g in groups or []:
                n += len(g.get("hooks", [])) if isinstance(g, dict) else 0
            hooks[ev] = hooks.get(ev, 0) + n
    w["hooks"] = hooks
    plugins = {**(settings.get("enabledPlugins") or {}), **(local.get("enabledPlugins") or {})}
    w["plugins_on"] = sorted(k for k, v in plugins.items() if v)
    cj = load_json(os.path.join(HOME, ".claude.json"))
    mcp = dict(cj.get("mcpServers") or {})
    proj = (cj.get("projects") or {}).get(cwd or os.getcwd(), {})
    mcp.update(proj.get("mcpServers") or {})
    mcp.update(load_json(os.path.join(cwd or os.getcwd(), ".mcp.json")).get("mcpServers") or {})
    w["mcp"] = sorted(mcp)
    w["output_style"] = settings.get("outputStyle")
    w["effort_env"] = os.environ.get("CLAUDE_EFFORT")
    return w


def machine_load():
    out = {"load": None, "cpus": os.cpu_count() or 1, "top": []}
    try:
        out["load"] = os.getloadavg()[0]
    except Exception:
        pass
    try:
        ps = subprocess.run(["ps", "-Ao", "pcpu,rss,comm"], capture_output=True, text=True, timeout=5).stdout.splitlines()[1:]
        rows = []
        for r in ps:
            parts = r.split(None, 2)
            if len(parts) == 3:
                rows.append((float(parts[0]), int(parts[1]) // 1024, os.path.basename(parts[2])))
        rows.sort(reverse=True)
        out["top"] = rows[:3]
    except Exception:
        pass
    return out


# --------------------------------------------------------------------------
# 4. Judge: turn numbers into ranked findings
# --------------------------------------------------------------------------
def guess_window(s):
    """1M for models that ship with it (Opus, Sonnet, Fable 5.x) or that ran past 200K; else 200K."""
    model = s["model"] or ""
    if "[1m]" in model or re.search(r"claude-(opus|sonnet|fable)-5", model):
        return 1_000_000
    if s["ctx_max"] > 200_000 or any((c[0] or 0) > 200_000 for c in s["compactions"]):
        return 1_000_000
    return 200_000


def judge(s, w, m, window):
    f = []  # (score, who, title, why, fix)

    def add(score, who, title, why, fix):
        f.append((score, who, title, why, fix))

    pct = 100 * s["ctx_now"] / window if window else 0
    if pct >= 75:
        add(100, "you", f"Context is {pct:.0f}% full", "Every turn re-reads the whole history. Replies get slow and fuzzy.", "Run /compact now. Or /clear and paste a short handoff.")
    elif pct >= 50:
        add(70, "you", f"Context is {pct:.0f}% full", "Each turn costs more as the history grows.", "Run /compact at the next calm moment.")
    big = [r for r in s["results"] if r[0] >= 20000]
    if big:
        total = sum(r[0] for r in big)
        top = big[0]
        add(80 if total > 200000 else 55, "me", f"{len(big)} huge tool result{'s' if len(big) != 1 else ''} (~{ktok(tok(total))} tokens)",
            f"Biggest: {top[1]} {human(top[0])} ({top[2]}). They stay in context forever.",
            "I will use offset/limit, rg, head, and subagents for big output.")
    rereads = [(p, n) for p, n in s["reads"].items() if n >= 3]
    if rereads:
        p, n = max(rereads, key=lambda x: x[1])
        add(50, "me", f"{len(rereads)} file{'s' if len(rereads) != 1 else ''} read 3+ times", f"Worst: {os.path.basename(p)} read {n} times.", "I will read once and keep notes. No more re-reads.")
    if s["slow"]:
        d, name, summ = s["slow"][0]
        add(65 if d > 120 else 45, "me", f"{len(s['slow'])} slow tool call{'s' if len(s['slow']) != 1 else ''} (over 20s)", f"Slowest: {name} {d:.0f}s ({summ}).", "I will run slow work in the background and add timeouts.")
    comp = s["compactions"]
    if len(comp) >= 2:
        add(60, "you", f"Compacted {len(comp)} times", "Session is too long. Each compact loses detail.", "Start a fresh session for the next task. Keep the handoff short.")
    if s["bytes"] > 15 * 1024 * 1024:
        add(40, "you", f"Transcript is {human(s['bytes'])}", "Resume and startup get slow on huge sessions.", "Finish this task, then start a new session.")
    cmd_total = sum(n for _, n in w["claude_md"]) + w["rules"][1]
    if cmd_total > 24000:
        add(60 if cmd_total > 48000 else 45, "you", f"CLAUDE.md + rules = ~{ktok(tok(cmd_total))} tokens", "Loaded on every turn, even when not needed.", "Trim or move rarely used parts into skills.")
    ab = s["attach_by_type"]
    servers = {re.match(r"mcp__(.+?)__", n).group(1) for n in s["mcp_tools"] if re.match(r"mcp__(.+?)__", n)}
    mcp_bytes = ab["deferred_tools_delta"] + ab["mcp_instructions_delta"]
    if len(s["mcp_tools"]) >= 100 or mcp_bytes > 60000:
        add(65, "you", f"{len(servers)} MCP servers, {len(s['mcp_tools'])} tools (~{ktok(tok(mcp_bytes))} tokens injected)",
            "Tool names and server rules are sent on every session start and cache miss.", "Disable MCP servers this project does not use.")
    elif len(w["mcp"]) >= 6:
        add(55, "you", f"{len(w['mcp'])} MCP servers connected", "Each server adds tool schemas to every turn.", "Disable servers you do not use in this project.")
    if ab["skill_listing"] > 40000:
        add(60, "you", f"Skill list injects ~{ktok(tok(ab['skill_listing']))} tokens", "Every skill name and description is sent each session.", "Disable plugins with many skills. Remove skills you never use.")
    if ab["hook_additional_context"] > 8000:
        add(45, "you", f"Start hooks inject ~{ktok(tok(ab['hook_additional_context']))} tokens", "A SessionStart hook pastes big text into every session.", "Shorten what the hook prints.")
    if len(w["plugins_on"]) >= 8:
        add(45, "you", f"{len(w['plugins_on'])} plugins enabled", "Each plugin adds skills, agents, and hooks.", "Turn off plugins you rarely use. It is reversible.")
    pre = w["hooks"].get("PreToolUse", 0) + w["hooks"].get("PostToolUse", 0)
    if pre >= 5:
        add(50, "you", f"{pre} tool hooks run on every tool call", "Each hook adds a process spawn before and after each tool.", "Check they are fast. Drop the ones you do not need.")
    if w["effort_env"] in ("high", "xhigh", "max") and s["tool_calls"].get("Read", 0) + s["tool_calls"].get("Bash", 0) > 0:
        add(35, "you", f"Effort is {w['effort_env']}", "High effort means long thinking before each step.", "Use /effort medium for simple work. Try /fast for quick tasks.")
    if s["turns_assistant"] and s["think_ms"] / 1000 > 600:
        add(40, "you", f"{s['think_ms'] / 60000:.0f} min spent thinking", "Long thinking slows every step.", "Lower effort for routine edits.")
    if s["errors"] >= 8:
        add(40, "me", f"{s['errors']} tool errors", "Retry loops waste turns.", "I will stop, read the error once, and change the plan.")
    if m["load"] and m["load"] > m["cpus"] * 0.8:
        top = ", ".join(f"{n} {c:.0f}%" for c, _, n in m["top"])
        add(50, "you", f"Mac is busy (load {m['load']:.1f} on {m['cpus']} cores)", f"Top: {top}.", "Quit the heavy app. Run ps:bag-mode to see more.")
    f.sort(reverse=True)
    return f


def verdict(f):
    top = f[0][0] if f else 0
    return "HIGH" if top >= 80 else "MEDIUM" if top >= 50 else "LOW"


# --------------------------------------------------------------------------
# 5. Report
# --------------------------------------------------------------------------
def report(s, w, m, window, f, as_json):
    pct = 100 * s["ctx_now"] / window if window else 0
    if as_json:
        print(json.dumps({
            "session": s["path"], "verdict": verdict(f), "context_tokens": s["ctx_now"],
            "window": window, "context_pct": round(pct, 1),
            "findings": [{"score": a, "who": b, "title": c, "why": d, "fix": e} for a, b, c, d, e in f],
        }, indent=2))
        return
    bar = "#" * int(pct / 5) + "." * (20 - int(pct / 5))
    age = ""
    if s["first_ts"] and s["last_ts"]:
        age = f", {(s['last_ts'] - s['first_ts']) / 3600:.1f}h old"
    print(f"SESSION  {os.path.basename(s['path'])}  ({human(s['bytes'])}{age})")
    print(f"MODEL    {s['model'] or '?'}   effort: {w['effort_env'] or '?'}   style: {w['output_style'] or 'default'}")
    print(f"CONTEXT  [{bar}] {pct:.0f}%  {ktok(s['ctx_now'])} of {ktok(window)} tokens")
    print(f"TURNS    {s['turns_assistant']} replies, {sum(s['tool_calls'].values())} tool calls, {len(s['compactions'])} compactions, {s['sidechain']} subagent lines")
    if s["tool_bytes"]:
        parts = ", ".join(f"{n} {human(b)}" for n, b in s["tool_bytes"].most_common(4))
        print(f"RESULTS  top tools by output: {parts}")
    ab = s["attach_by_type"]
    if ab:
        top = ", ".join(f"{k} ~{ktok(tok(v))}t" for k, v in ab.most_common(4))
        print(f"STARTUP  injected: {top}")
    print(f"STATIC   CLAUDE.md+rules ~{ktok(tok(sum(n for _, n in w['claude_md']) + w['rules'][1]))}t | "
          f"{len(w['plugins_on'])} plugins on | {sum(w['hooks'].values())} hooks | {len(s['mcp_tools'])} MCP tools")
    print()
    print(f"VERDICT  {verdict(f)}")
    if not f:
        print("Nothing slow found. The wait is the model, not the setup. Try /fast.")
        return
    print()
    for i, (score, who, title, why, fix) in enumerate(f[:7], 1):
        print(f"{i}. [{who.upper()}] {title}")
        print(f"   why: {why}")
        print(f"   fix: {fix}")
    print()
    print("[ME] = Claude can do it now. [YOU] = only you can: slash commands and settings.")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--session", help="session id (or prefix) or path to a .jsonl file")
    ap.add_argument("--all", action="store_true", help="pick the newest session of any project")
    ap.add_argument("--window", type=int, help="context window in tokens (default: guessed)")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--brief", action="store_true", help="3 SPEED lines for whatsleft-scan, nothing else")
    a = ap.parse_args()

    path = find_session(a.session, a.all)
    if not path:
        print("No session transcript found under ~/.claude/projects.", file=sys.stderr)
        sys.exit(1)
    s = scan_transcript(path)
    window = a.window or guess_window(s)
    w = static_weight(s["cwd"])
    m = machine_load()
    f = judge(s, w, m, window)
    if a.brief:
        for score, who, title, _why, fix in [x for x in f if x[0] >= 60][:3]:
            print(f"  SPEED [{who.upper()}] {title} — {fix}")
        return
    report(s, w, m, window, f, a.json)


if __name__ == "__main__":
    main()
