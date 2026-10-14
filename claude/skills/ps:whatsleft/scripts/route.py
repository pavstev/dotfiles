#!/usr/bin/env python3
"""route.py: decide who does each what's-left row, and what comes next.

The agent judges each row once (its kind, its risk). This script does the rest
the same way every time: DO, ASK, YOU, WAIT, KEEP, SKIP or DONE per row, the question
list for ps:askme (most costly first, capped), and the next step.

  python3 route.py rows.json            # or: ... < rows.json
  python3 route.py --kill rows.json     # `kill dead tasks`: STOP tasks are DO
  python3 route.py --json rows.json     # machine output

rows.json is a JSON list. Each row:
  id     "m1"            unique, short
  text   "No test for the export"
  kind   work | decision | optional | value | task | login | secret | payment | refused | no-tool
  who    me | you        only for kind optional (default me)
  risk   push | deploy | delete | paid | prod-config   (work only; omit when safe)
  asked  true            the user asked for exactly this in the session
  after  ["y1"]          ids this row waits on
  stop   true            task only: the scan says STOP
  tried  "Chrome: AWS sign-in page"   required for no-tool
  done   true            proof seen this session
  skip   true            the user said skip (or you took the default: skip)

Exit 0 ok, 2 bad input (the reason is printed). Stdlib only, Python 3.9+.
"""
import argparse
import json
import sys

KINDS = {"work", "decision", "optional", "value", "task", "login", "secret",
         "payment", "refused", "no-tool"}
RISKS = {"push", "deploy", "delete", "paid", "prod-config"}
USER_ONLY = {"login", "secret", "payment", "refused", "no-tool"}
CAP = 4  # AskUserQuestion takes at most 4 questions per call


class BadInput(Exception):
    pass


def validate(rows):
    if not isinstance(rows, list):
        raise BadInput("rows must be a JSON list")
    ids = set()
    for i, r in enumerate(rows):
        where = "row %d" % (i + 1)
        if not isinstance(r, dict):
            raise BadInput("%s: not an object" % where)
        rid = r.get("id")
        if not rid or not isinstance(rid, str):
            raise BadInput("%s: id missing" % where)
        if rid in ids:
            raise BadInput("%s: duplicate id %s" % (where, rid))
        ids.add(rid)
        if not r.get("text"):
            raise BadInput("%s: text missing" % rid)
        kind = r.get("kind")
        if kind not in KINDS:
            raise BadInput("%s: kind %r not in %s" % (rid, kind, sorted(KINDS)))
        risk = r.get("risk")
        if risk is not None and risk not in RISKS:
            raise BadInput("%s: risk %r not in %s" % (rid, risk, sorted(RISKS)))
        if risk is not None and kind != "work":
            raise BadInput("%s: risk only goes on kind work" % rid)
        if r.get("who", "me") not in ("me", "you"):
            raise BadInput("%s: who must be me or you" % rid)
        if kind == "no-tool" and not str(r.get("tried", "")).strip():
            raise BadInput("%s: no-tool needs 'tried': name the tool you tried first" % rid)
        after = r.get("after", [])
        if not isinstance(after, list):
            raise BadInput("%s: after must be a list" % rid)
    for r in rows:
        for dep in r.get("after", []):
            if dep not in ids:
                raise BadInput("%s: after names unknown id %s" % (r["id"], dep))
            if dep == r["id"]:
                raise BadInput("%s: waits on itself" % r["id"])


def first_route(r, kill):
    """Route a row on its own fields, before dependencies."""
    if r.get("done"):
        return "DONE"
    if r.get("skip"):
        return "SKIP"
    kind = r["kind"]
    if kind in USER_ONLY:
        return "YOU"
    if kind in ("decision", "value"):
        return "ASK"
    if kind == "optional":
        return "YOU" if r.get("who") == "you" else "ASK"
    if kind == "task":
        if not r.get("stop"):
            return "KEEP"
        return "DO" if kill else "ASK"
    # work
    if r.get("risk") and not r.get("asked"):
        return "ASK"
    return "DO"


def priority(r):
    """Lower asks first: costly decisions, then tasks, then optional rows."""
    if r["kind"] == "optional":
        return 2
    if r["kind"] == "task":
        return 1
    return 0


def route(rows, kill=False, cap=CAP):
    validate(rows)
    by_id = {r["id"]: r for r in rows}
    out = {r["id"]: first_route(r, kill) for r in rows}
    # A DO row waits on any row not DONE or SKIP. A question that needs the
    # answer to another question is asked next round (askme: dependent
    # questions go alone).
    closed = ("DONE", "SKIP")
    for r in rows:
        deps = [out[d] for d in r.get("after", [])]
        if out[r["id"]] == "DO" and any(d not in closed for d in deps):
            out[r["id"]] = "WAIT"
        elif out[r["id"]] == "ASK" and "ASK" in deps:
            out[r["id"]] = "WAIT"
    ask = sorted((r for r in rows if out[r["id"]] == "ASK"),
                 key=lambda r: priority(r))  # sorted() is stable: input order breaks ties
    asked, defaulted = ask[:cap], ask[cap:]
    for r in defaulted:
        out[r["id"]] = "DEFAULT"
    do = [r["id"] for r in rows if out[r["id"]] == "DO"]
    waits_on_do = any(
        out[r["id"]] == "WAIT" and any(out[d] == "DO" for d in r.get("after", []))
        for r in rows)
    if asked:
        then = "rerun"     # after the answers: update the rows, run route.py again
    elif waits_on_do:
        then = "rerun"     # the DO rows unlock more work
    else:
        then = "mic-drop"  # nothing left to ask: close out with ps:mic-drop
    return {
        "routes": [{"id": r["id"], "route": out[r["id"]], "text": r["text"]} for r in rows],
        "do": do,
        "ask": [r["id"] for r in asked],
        "default": [r["id"] for r in defaulted],
        "you": [r["id"] for r in rows if out[r["id"]] == "YOU"],
        "then": then,
        "_by_id": by_id,
    }


def render(res):
    lines = []
    for x in res["routes"]:
        lines.append("%-7s %-4s %s" % (x["route"], x["id"], x["text"]))
    lines.append("")
    lines.append("NEXT 1 do now: %s" % (", ".join(res["do"]) or "nothing"))
    if res["ask"]:
        lines.append("NEXT 2 ps:askme: whatsleft: " + "; ".join(
            "%s %s" % (i, res["_by_id"][i]["text"]) for i in res["ask"]))
    else:
        lines.append("NEXT 2 ps:askme: nothing to ask")
    if res["default"]:
        lines.append("       your pick, named in one line: %s" % ", ".join(res["default"]))
    if res["then"] == "mic-drop":
        lines.append("NEXT 3 run ps:mic-drop")
    else:
        lines.append("NEXT 3 update the rows (answers, done), run route.py again")
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("file", nargs="?", help="rows JSON (default: stdin)")
    ap.add_argument("--kill", action="store_true", help="`kill dead tasks`: STOP tasks are DO")
    ap.add_argument("--cap", type=int, default=CAP, help="max questions (default 4)")
    ap.add_argument("--json", action="store_true", help="print JSON")
    a = ap.parse_args(argv)
    try:
        raw = open(a.file, encoding="utf-8").read() if a.file else sys.stdin.read()
        res = route(json.loads(raw), kill=a.kill, cap=a.cap)
    except (BadInput, ValueError) as e:
        print("route.py: bad input: %s" % e)
        return 2
    if a.json:
        res.pop("_by_id")
        print(json.dumps(res, indent=2, ensure_ascii=False))
    else:
        print(render(res))
    return 0


if __name__ == "__main__":
    sys.exit(main())
