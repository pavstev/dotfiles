#!/usr/bin/env python3
"""harness_stats.py: measure Claude Code sessions from local transcripts. Stdlib only, Python 3.9+.

  python3 harness_stats.py collect [--since 2026-10-01] [--out FILE] [--exclude SID ...]
  python3 harness_stats.py cuts    [--repo ~/dotfiles | --gh OWNER/REPO] [--since DATE] [--path claude]
  python3 harness_stats.py report  [--in FILE] [--cut ISO ...] [--by period|project]
  python3 harness_stats.py errors  [--in FILE] [--project NAME] [--top 12] [--daily]
  python3 harness_stats.py skills  [--in FILE]
  python3 harness_stats.py inventory [--in FILE] [--top 8]
  python3 harness_stats.py agents  [--in FILE] [--cut ISO ...]

collect reads ~/.claude/projects/*/*.jsonl (main sessions), their subagents/*.jsonl and
.meta.json, and the desktop app's session records (title, archived). It writes one JSON
row per main session. Everything else reads that file. Default file:
$TMPDIR/harness_stats.json.

Row keys: sid, project, folder, start, end (ISO UTC), title, archived, effort, cli (Claude Code
version), model, first_ctx,
peak_ctx (tokens), active_min, wall_min, stats {calls, out, fresh_in, cache_read, prompts,
interrupts, corrections, tool_errors, hook_denials, compactions, api_errors, from_handoff},
tools, skills, bash (command heads), errors (normalized text -> count, subagents included),
agent_calls [{type, model, effort, bg, plen}], sub {n, calls, fresh_in, cache_read, out, tool_errors,
models, effort, spawns [[agent type, start ctx], ...]}. A guard or hook deny counts as hook_denials, not tool_errors.
"""
import argparse
import collections
import glob
import json
import os
import re
import statistics
import subprocess
import sys
from datetime import datetime, timedelta, timezone

HOME = os.path.expanduser("~")
PROJECTS = os.path.join(HOME, ".claude", "projects")
DESKTOP = os.path.join(HOME, "Library", "Application Support", "Claude", "claude-code-sessions")
DEFAULT_OUT = os.path.join(os.environ.get("TMPDIR", "/tmp"), "harness_stats.json")

INTERRUPT = "[Request interrupted by user"
# Corrections only. "do not stop until", "stop the task" are instructions, not corrections.
CORRECTION = re.compile(
    r"\b(why did you|that'?s wrong|is wrong|again\?|you didn'?t|not what i (asked|wanted|said)|ugh|wtf"
    r"|still (not|broken|wrong|failing)|as i said|i said|i told you|i already (said|told))\b", re.I)
HANDOFF = re.compile(r"Read \S+\.md and do it")
POLL_TOOLS = ("ScheduleWakeup", "CronCreate", "Monitor")


def ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def project_name(folder):
    """Folder name under ~/.claude/projects -> short project name (worktrees fold into their repo)."""
    p = re.sub(r"--claude-worktrees-.*", "", folder)
    home = "-" + HOME.strip("/").replace("/", "-")
    if p == home:
        return "~"
    if "scratch-workspaces" in p:
        return "scratch"
    if p.startswith("-private-"):
        return "tmp"
    p = p[len(home) + 1:] if p.startswith(home + "-") else p.lstrip("-")
    return re.sub(r"^work-", "", p) or "?"


def desktop_meta():
    meta = {}
    for f in glob.glob(os.path.join(DESKTOP, "*", "*", "local_*.json")):
        try:
            d = json.load(open(f))
        except (OSError, ValueError):
            continue
        if d.get("cliSessionId"):
            meta[d["cliSessionId"]] = {"title": d.get("title"), "archived": bool(d.get("isArchived")),
                                       "effort": d.get("effort"), "perm": d.get("permissionMode")}
    return meta


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(x.get("text", "") for x in content if isinstance(x, dict) and x.get("type") == "text")
    return ""


def norm_error(name, content):
    t = content if isinstance(content, str) else json.dumps(content)
    t = re.sub(r"/private/tmp/\S+|/Users/[^\s/]+/\S*", "<path>", t)
    t = re.sub(r"\d+", "N", t).replace("\n", " ")
    return "%s: %s" % (name, t[:110])


def scan(path, is_sub=False):
    """One transcript file -> dict of counters and lists."""
    r = collections.Counter()
    tools, skills, bash, errors = (collections.Counter() for _ in range(4))
    agents, times, seen, models = [], [], set(), collections.Counter()
    first_ctx, peak, names, version = None, 0, {}, None
    for line in open(path, errors="ignore"):
        try:
            o = json.loads(line)
        except ValueError:
            continue
        if o.get("timestamp"):
            try:
                times.append(ts(o["timestamp"]))
            except ValueError:
                pass
        kind, msg = o.get("type"), o.get("message") or {}
        version = version or o.get("version")
        if kind == "system":
            if o.get("subtype") == "compact_boundary":
                r["compactions"] += 1
            elif o.get("subtype") == "api_error":
                r["api_errors"] += 1
        elif kind == "assistant":
            for c in msg.get("content") or []:
                if not isinstance(c, dict) or c.get("type") != "tool_use":
                    continue
                n, inp = c.get("name", ""), c.get("input") or {}
                names[c.get("id")] = n
                tools[n] += 1
                if n == "Skill":
                    skills[inp.get("skill", "?")] += 1
                elif n in ("Agent", "Task"):
                    agents.append({"type": inp.get("subagent_type") or "general-purpose", "model": inp.get("model"), "effort": inp.get("effort"),
                                   "bg": inp.get("run_in_background"), "plen": len(inp.get("prompt") or "")})
                elif n == "Bash":
                    cmd = re.sub(r"^(cd [^&;]+(&&|;)\s*)+", "", (inp.get("command") or "").strip()).split()
                    two = ("git", "npm", "pnpm", "bun", "go", "uv", "npx", "gh", "wrangler", "python3", "node", "make", "cargo")
                    if cmd:
                        bash[" ".join(cmd[:2]) if cmd[0] in two else cmd[0][:24]] += 1
            mid, u = msg.get("id"), msg.get("usage") or {}
            if mid in seen or not u or msg.get("model") == "<synthetic>":
                continue
            seen.add(mid)
            t_in, t_out = u.get("input_tokens") or 0, u.get("output_tokens") or 0
            t_cr, t_cc = u.get("cache_read_input_tokens") or 0, u.get("cache_creation_input_tokens") or 0
            ctx = t_in + t_cr + t_cc
            models[msg.get("model")] += 1
            r["calls"] += 1
            r["out"] += t_out
            r["fresh_in"] += t_in + t_cc
            r["cache_read"] += t_cr
            if first_ctx is None and ctx:
                first_ctx = ctx
            peak = max(peak, ctx)
        elif kind == "user":
            content = msg.get("content")
            if isinstance(content, list) and any(isinstance(x, dict) and x.get("type") == "tool_result" for x in content):
                for x in content:
                    if isinstance(x, dict) and x.get("type") == "tool_result" and x.get("is_error"):
                        e = norm_error(names.get(x.get("tool_use_id"), "?"), x.get("content"))
                        r["hook_denials" if "hook error" in e else "tool_errors"] += 1
                        errors[e] += 1
                continue
            if o.get("isMeta") or o.get("isCompactSummary") or (o.get("isSidechain") and not is_sub):
                continue
            t = text_of(content)
            if t.startswith(INTERRUPT):
                r["interrupts"] += 1
                continue
            if not t.strip() or (t.lstrip().startswith("<") and "command-name" not in t):
                continue
            r["prompts"] += 1
            if CORRECTION.search(t):
                r["corrections"] += 1
            if r["prompts"] == 1 and HANDOFF.search(t):
                r["from_handoff"] = 1
    times.sort()
    active = sum(g for g in ((b - a).total_seconds() for a, b in zip(times, times[1:])) if g < 600)
    return {"r": r, "tools": tools, "skills": skills, "bash": bash, "errors": errors, "agents": agents,
            "times": times, "first_ctx": first_ctx or 0, "peak": peak, "models": models, "active_s": active,
            "version": version}


def cmd_collect(a):
    since = ts(a.since + "T00:00:00+00:00") if "T" not in a.since else ts(a.since)
    meta, rows = desktop_meta(), []
    for f in glob.glob(os.path.join(PROJECTS, "*", "*.jsonl")):
        sid = os.path.basename(f)[:-6]
        if sid in a.exclude or os.path.getmtime(f) < since.timestamp():
            continue
        s = scan(f)
        if not s["times"] or s["times"][0] < since or not s["r"]["calls"]:
            continue
        folder = os.path.basename(os.path.dirname(f))
        sub = {"n": 0, "calls": 0, "fresh_in": 0, "cache_read": 0, "out": 0, "tool_errors": 0,
               "models": collections.Counter(), "effort": collections.Counter(), "spawns": []}
        errors = collections.Counter(s["errors"])
        for sf in glob.glob(os.path.join(os.path.dirname(f), sid, "subagents", "*.jsonl")):
            ss = scan(sf, is_sub=True)
            sub["n"] += 1
            for k in ("calls", "fresh_in", "cache_read", "out", "tool_errors"):
                sub[k] += ss["r"][k]
            errors.update(ss["errors"])
            md = {}
            try:
                md = json.load(open(sf[:-6] + ".meta.json"))
                sub["models"][md.get("model") or "inherit"] += 1
                sub["effort"][md.get("effort") or "unset"] += 1
            except (OSError, ValueError):
                pass
            if ss["first_ctx"]:
                sub["spawns"].append([md.get("agentType") or "?", ss["first_ctx"]])
        m = meta.get(sid, {})
        rows.append({
            "sid": sid, "project": project_name(folder), "folder": folder,
            "start": s["times"][0].isoformat(), "end": s["times"][-1].isoformat(),
            "title": m.get("title"), "archived": m.get("archived"), "effort": m.get("effort"),
            "cli": s["version"],
            "model": s["models"].most_common(1)[0][0] if s["models"] else None,
            "first_ctx": s["first_ctx"], "peak_ctx": s["peak"],
            "active_min": round(s["active_s"] / 60, 1),
            "wall_min": round((s["times"][-1] - s["times"][0]).total_seconds() / 60, 1),
            "stats": dict(s["r"]), "tools": dict(s["tools"]), "skills": dict(s["skills"]),
            "bash": dict(s["bash"].most_common(15)), "errors": dict(errors.most_common(20)),
            "agent_calls": s["agents"],
            "sub": {k: (dict(v) if isinstance(v, collections.Counter) else v) for k, v in sub.items()},
        })
    json.dump(rows, open(a.out, "w"))
    print("%d sessions -> %s" % (len(rows), a.out))


def cmd_cuts(a):
    """List harness pushes: commits touching --path, grouped when less than 3 h apart (a push group)."""
    if a.gh:
        since = a.since if "T" in a.since else a.since + "T00:00:00Z"
        out = subprocess.run(["gh", "api", "repos/%s/commits?since=%s&path=%s&per_page=100" % (a.gh, since, a.path),
                              "--paginate", "--jq", '.[] | "\\(.commit.committer.date) \\(.sha[0:7]) \\(.commit.message|split("\\n")[0])"'],
                             capture_output=True, text=True).stdout.strip().splitlines()
    else:
        repo = os.path.expanduser(a.repo)
        out = subprocess.run(["git", "-C", repo, "log", "--since", a.since, "--format=%cI %h %s", "--", a.path],
                             capture_output=True, text=True).stdout.strip().splitlines()
        root = subprocess.run(["git", "-C", repo, "log", "--reverse", "--format=%cI", "--max-count=1"],
                              capture_output=True, text=True).stdout.strip()
        first = subprocess.run(["git", "-C", repo, "rev-list", "--max-parents=0", "--format=%cI", "HEAD"],
                               capture_output=True, text=True).stdout.strip().splitlines()
        root = first[-1] if first else root
        print("Repo history starts %s. If that is inside your window, the older pushes are in another repo:" % root)
        print("  gh repo list --limit 50   then   harness_stats.py cuts --gh OWNER/REPO --since %s\n" % a.since)
    commits = sorted((ts(l.split()[0]), l.split(None, 1)[1]) for l in out if l)
    groups = []
    for t, s in commits:
        if groups and t - groups[-1][-1][0] < timedelta(hours=3):
            groups[-1].append((t, s))
        else:
            groups.append([(t, s)])
    for g in groups:
        print("%s  %2d commits  %s" % (g[0][0].isoformat(), len(g), g[0][1][:70]))
        for t, s in g[1:4]:
            print("    %s" % s[:76])
    print("\nUse the first time of a group as --cut. Commits from an older history are missing here.")


def load(a):
    return json.load(open(a.inp))


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else 0


def summarize(rows):
    n = len(rows) or 1
    g = lambda k: sum(r["stats"].get(k, 0) for r in rows)
    prompts, tools = g("prompts") or 1, sum(sum(r["tools"].values()) for r in rows) or 1
    agents = [c for r in rows for c in r["agent_calls"]]
    fresh, cread = g("fresh_in"), g("cache_read")
    sub_calls = sum(r["sub"]["calls"] for r in rows)
    return collections.OrderedDict([
        ("sessions", len(rows)),
        ("archived", sum(1 for r in rows if r.get("archived"))),
        ("start ctx med K", round(med([r["first_ctx"] for r in rows]) / 1000)),
        ("peak ctx med K", round(med([r["peak_ctx"] for r in rows]) / 1000)),
        ("peak > 500K", sum(1 for r in rows if r["peak_ctx"] > 500000)),
        ("compactions", g("compactions")),
        ("prompts/session med", med([r["stats"].get("prompts", 0) for r in rows])),
        ("questions/session", round(sum(r["tools"].get("AskUserQuestion", 0) for r in rows) / n, 2)),
        ("questions/100 prompts", round(100 * sum(r["tools"].get("AskUserQuestion", 0) for r in rows) / prompts, 1)),
        ("stops/100 prompts", round(100 * g("interrupts") / prompts, 1)),
        ("corrections/100 prompts", round(100 * g("corrections") / prompts, 1)),
        ("tool errors/100 calls", round(100 * g("tool_errors") / tools, 1)),
        ("hook denials", g("hook_denials")),
        ("cache hit %", round(100 * cread / ((cread + fresh) or 1))),
        ("wall min med", round(med([r["wall_min"] for r in rows]))),
        ("active min med", round(med([r["active_min"] for r in rows]))),
        ("subagents/session", round(sum(r["sub"]["n"] for r in rows) / n, 1)),
        ("subagent share of calls %", round(100 * sub_calls / ((sub_calls + g("calls")) or 1))),
        ("agent calls w/ effort %", round(100 * sum(1 for c in agents if c["effort"]) / (len(agents) or 1))),
        ("general-purpose spawns %", round(100 * sum(1 for c in agents if c.get("type") == "general-purpose")
                                           / (len([c for c in agents if c.get("type")]) or 1))),
        ("wait calls (Monitor, Cron, Wakeup)", sum(r["tools"].get(t, 0) for r in rows for t in POLL_TOOLS)),
        ("from hand-off", g("from_handoff")),
        ("Claude Code versions", "%s..%s" % (min(vs), max(vs)) if (vs := [r.get("cli") for r in rows if r.get("cli")]) else "?"),
    ])


def table(groups):
    keys = list(groups)
    stats = {k: summarize(v) for k, v in groups.items()}
    metrics = list(stats[keys[0]]) if keys else []
    print("| metric | " + " | ".join(keys) + " |")
    print("|---|" + "---|" * len(keys))
    for m in metrics:
        print("| %s | %s |" % (m, " | ".join(str(stats[k][m]) for k in keys)))


def cmd_report(a):
    rows = load(a)
    if a.by == "project":
        groups = collections.defaultdict(list)
        for r in rows:
            groups[r["project"]].append(r)
        groups = collections.OrderedDict(sorted(groups.items(), key=lambda kv: -len(kv[1]))[:a.top])
    else:
        cuts, groups = periods(rows, a.cut)
        print("Periods: P0 < " + " <= ".join("P%d %s" % (i + 1, c.isoformat()) for i, c in enumerate(cuts)) + "\n")
    table(groups)
    print("\nHigher is better: cache hit %, agent calls w/ effort %, from hand-off. Lower is better for the rest.")
    small = [k for k, v in groups.items() if len(v) < 20]
    if small:
        print("\nEarly (under 20 sessions): %s. Read their numbers as a hint, not a result." % ", ".join(small))


def periods(rows, cut):
    cuts = sorted(ts(c) for c in cut)
    groups = collections.OrderedDict(("P%d" % i, []) for i in range(len(cuts) + 1))
    for r in rows:
        groups["P%d" % sum(1 for c in cuts if ts(r["start"]) >= c)].append(r)
    return cuts, groups


def cmd_agents(a):
    """Dispatch watch: subagent spawns per agent type and period, with share and start context."""
    cuts, groups = periods(load(a), a.cut)
    data = collections.OrderedDict()
    for k, rows in groups.items():
        by = collections.defaultdict(list)
        for r in rows:
            for typ, ctx in (r.get("sub") or {}).get("spawns", []):
                by[typ].append(ctx)
        data[k] = by
    types = sorted({t for by in data.values() for t in by}, key=lambda t: -sum(len(by.get(t, [])) for by in data.values()))
    print("Periods: P0 < " + " <= ".join("P%d %s" % (i + 1, c.isoformat()) for i, c in enumerate(cuts)) + "\n")
    print("| agent type | " + " | ".join("%s spawns (share) | %s start K" % (k, k) for k in data) + " |")
    print("|---|" + "---|---|" * len(data))
    for t in types[:a.top]:
        cells = []
        for k, by in data.items():
            total = sum(len(v) for v in by.values()) or 1
            xs = by.get(t, [])
            cells.append("%d (%d%%) | %s" % (len(xs), round(100 * len(xs) / total), round(med(xs) / 1000) if xs else "-"))
        print("| %s | %s |" % (t, " | ".join(cells)))
    print("\nSessions: " + ", ".join("%s %d" % (k, len(v)) for k, v in groups.items()))
    print("Start K = median first-turn context of the spawn. Watch: general-purpose share should fall and")
    print("task-implementer / worker should rise after a dispatch change. Agents with a tools: list start near 15K.")
    small = [k for k, v in groups.items() if len(v) < 20]
    if small:
        print("Early (under 20 sessions): %s. Read their numbers as a hint, not a result." % ", ".join(small))


def cmd_errors(a):
    by = collections.defaultdict(collections.Counter)
    rows = [r for r in load(a) if not a.project or r["project"] == a.project]
    if a.daily:
        days = collections.defaultdict(collections.Counter)
        total = collections.Counter()
        for r in rows:
            days[r["start"][:10]].update(r["errors"])
            total.update(r["errors"])
        top = [e for e, _ in total.most_common(a.top)]
        dates = sorted(days)
        print("| cluster | " + " | ".join(d[5:] for d in dates) + " |")
        print("|---|" + "---|" * len(dates))
        for e in top:
            print("| %s | %s |" % (e[:70].replace("|", "/"), " | ".join(str(days[d].get(e, 0)) for d in dates)))
        return
    for r in rows:
        by[r["project"]].update(r["errors"])
    for p, c in sorted(by.items(), key=lambda kv: -sum(kv[1].values())):
        print("\n## %s (%d errors)" % (p, sum(c.values())))
        for e, n in c.most_common(a.top):
            print("%5d  %s" % (n, e))


def cmd_skills(a):
    used = collections.Counter()
    for r in load(a):
        used.update(r["skills"])
    installed = set()
    roots = [os.path.join(HOME, ".claude", "skills")] + glob.glob(os.path.join(HOME, "*", ".claude", "skills")) \
        + glob.glob(os.path.join(HOME, "work", "*", ".claude", "skills"))
    for d in roots:
        for s in glob.glob(os.path.join(d, "*", "SKILL.md")):
            installed.add(os.path.basename(os.path.dirname(s)))
    print("Used (Skill tool calls):")
    for s, n in used.most_common():
        print("%5d  %s" % (n, s))
    print("\nNames above that no longer exist were renamed or removed; check the skills repo history.")
    print("Plugin skills and skillOverrides are not covered.")
    print("\nInstalled in ~/.claude/skills, ~/*/.claude/skills or ~/work/*/.claude/skills, never called:")
    for s in sorted(installed - set(used)):
        print("       %s" % s)


def cmd_inventory(a):
    """Per project: CLAUDE.md size, rules (path-scoped or not), agents, skills, hooks, sessions."""
    rows = load(a)
    count = collections.Counter(r["project"] for r in rows)
    folders = {}
    for r in rows:
        folders.setdefault(r["project"], r["folder"])
    print("| project | sessions | CLAUDE.md lines | rules (paths/all) | agents | skills | project hooks |")
    print("|---|---|---|---|---|---|---|")
    for p, n in count.most_common(a.top):
        if p in ("tmp", "scratch"):
            continue
        cands = [os.path.join(HOME, "work", p), os.path.join(HOME, p)] if p not in ("~", "tmp", "scratch") else [HOME]
        root = next((c for c in cands if os.path.isdir(c)), None)
        if not root:
            print("| %s | %d | (folder gone) | | | | |" % (p, n))
            continue
        md = os.path.join(root, "CLAUDE.md") if p != "~" else os.path.join(HOME, ".claude", "CLAUDE.md")
        lines = sum(1 for _ in open(md, errors="ignore")) if os.path.isfile(md) else 0
        rules = glob.glob(os.path.join(root, ".claude", "rules", "*.md")) if p != "~" else []
        scoped = sum(1 for f in rules if re.search(r"^paths:", open(f, errors="ignore").read(600), re.M))
        agents = len(glob.glob(os.path.join(root, ".claude", "agents", "*.md"))) if p != "~" else 0
        skills = len(glob.glob(os.path.join(root, ".claude", "skills", "*", "SKILL.md"))) if p != "~" else 0
        hooks = ""
        sj = os.path.join(root, ".claude", "settings.json")
        if p != "~" and os.path.isfile(sj):
            try:
                hooks = ",".join(sorted((json.load(open(sj)).get("hooks") or {}).keys()))
            except ValueError:
                hooks = "(bad json)"
        print("| %s | %d | %s | %d/%d | %d | %d | %s |" % (p, n, lines or "none", scoped, len(rules), agents, skills, hooks or "-"))


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = p.add_subparsers(dest="cmd", required=True)
    c = sp.add_parser("collect")
    c.add_argument("--since", default=(datetime.now(timezone.utc) - timedelta(days=14)).strftime("%Y-%m-%d"))
    c.add_argument("--out", default=DEFAULT_OUT)
    c.add_argument("--exclude", nargs="*", default=[])
    c = sp.add_parser("cuts")
    c.add_argument("--repo", default="~/dotfiles")
    c.add_argument("--gh", help="OWNER/REPO on GitHub, read with gh api (for an archived older history)")
    c.add_argument("--since", default="3 weeks ago")
    c.add_argument("--path", default="claude")
    for name in ("report", "errors", "skills", "inventory", "agents"):
        c = sp.add_parser(name)
        c.add_argument("--in", dest="inp", default=DEFAULT_OUT)
        c.add_argument("--top", type=int, default=12)
        if name in ("report", "agents"):
            c.add_argument("--cut", nargs="*", default=[])
        if name == "report":
            c.add_argument("--by", choices=("period", "project"), default="period")
        if name == "errors":
            c.add_argument("--project")
            c.add_argument("--daily", action="store_true", help="cluster x day table: when did it start or stop")
    a = p.parse_args(argv)
    {"collect": cmd_collect, "cuts": cmd_cuts, "report": cmd_report, "errors": cmd_errors, "skills": cmd_skills,
     "inventory": cmd_inventory, "agents": cmd_agents}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
