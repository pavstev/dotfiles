#!/usr/bin/env python3
"""check.py — lint every skill folder in this directory against the rules we rely on.

Rules come from the Claude Code skills docs, Anthropic's skill-authoring best
practices, and the agentskills.io spec. Stdlib only. Exit 1 on any FAIL.

  python3 claude/skills/check.py              # all skills
  python3 claude/skills/check.py ps:askme     # one or more skill folders
  python3 claude/skills/check.py --self-test  # run the built-in fixture tests
  python3 claude/skills/check.py -q           # failures only, nothing when all pass
"""
import os
import re
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))

# Frontmatter keys Claude Code documents. Anything else is ignored by Claude
# Code, so it is almost always a typo.
KNOWN_KEYS = {
    "name", "description", "when_to_use", "argument-hint", "arguments",
    "disable-model-invocation", "user-invocable", "allowed-tools",
    "disallowed-tools", "model", "effort", "context", "agent", "background",
    "hooks", "paths", "shell", "metadata", "license", "compatibility",
}
NAME_RE = re.compile(r"^(ps:)?[a-z0-9]+(-[a-z0-9]+)*$")  # spec regex + our ps: prefix
MAX_NAME = 64
MAX_DESC = 1024          # spec / Claude Code hard cap
MAX_DESC_COMBINED = 1536 # description + when_to_use, truncated past this in the listing
MAX_BODY_LINES = 500     # Anthropic: keep SKILL.md body under 500 lines
MAX_BODY_TOKENS = 5000   # agentskills.io: < 5000 tokens recommended (4 chars/token)
WARN_BODY_TOKENS = 2500  # our own bar: a skill this size is doing too much
JUNK = {".ruff_cache", "__pycache__", ".pytest_cache", ".DS_Store", ".mypy_cache"}
HARDCODED = re.compile(r"(~|\$HOME|/(?:Users|home)/[^/\s]+)/\.claude/skills/")
XML_TAG = re.compile(r"<[a-zA-Z/][^>]*>")
MD_LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
SKILL_DIR_REF = re.compile(r"\$\{CLAUDE_SKILL_DIR\}/([^\s`'\")*]+)")
FORCE_LOAD = re.compile(r"(^|\s)@(~/|/|\./|skills/)")
INTERPRETERS = ("bash", "sh", "zsh", "python3", "python", "node", "uv", "bun")


class Report:
    def __init__(self, skill):
        self.skill = skill
        self.fails = []
        self.warns = []

    def fail(self, msg):
        self.fails.append(msg)

    def warn(self, msg):
        self.warns.append(msg)


def parse_frontmatter(text):
    """Return (fields, body, errors). Strict on purpose: it mimics the parsers
    that silently drop a skill, instead of forgiving what they would not."""
    errors = []
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        return {}, text, ["frontmatter must start on line 1 with ---"]
    try:
        end = next(i for i in range(1, len(lines)) if lines[i].strip() == "---")
    except StopIteration:
        return {}, text, ["frontmatter never closes (missing second ---)"]
    fields = {}
    for raw in lines[1:end]:
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        if raw[0] in " \t":
            continue  # nested value (hooks:, metadata:) — not validated here
        if ":" not in raw:
            errors.append("frontmatter line is not key: value → %r" % raw)
            continue
        key, _, value = raw.partition(":")
        key, value = key.strip(), value.strip()
        fields[key] = value
        if not value:
            continue
        q = value[0]
        if q in ('"', "'"):
            if len(value) < 2 or value[-1] != q:
                errors.append("%s: quoted value does not close" % key)
            continue
        if ": " in value or value.endswith(":"):
            errors.append("%s: unquoted value contains ': ' — quote the whole value" % key)
        if value[0] in "[]{}&*!|>%@`":
            errors.append("%s: unquoted value starts with %r — quote it" % (key, value[0]))
        if " #" in value:
            errors.append("%s: unquoted ' #' starts a YAML comment — quote the value" % key)
        if " ... " in value:
            errors.append("%s: ' ... ' breaks Claude Code's parser — rephrase or quote" % key)
    body = "\n".join(lines[end + 1:])
    return fields, body, errors


def unquote(v):
    if len(v) >= 2 and v[0] in ('"', "'") and v[-1] == v[0]:
        return v[1:-1]
    return v


def check_skill(path):
    name = os.path.basename(path.rstrip("/"))
    r = Report(name)
    skill_md = os.path.join(path, "SKILL.md")
    if not os.path.isfile(skill_md):
        r.fail("no SKILL.md")
        return r

    # junk that must never ship
    for root, dirs, files in os.walk(path):
        for j in sorted(set(dirs + files) & JUNK):
            r.fail("junk in skill folder: %s" % os.path.relpath(os.path.join(root, j), path))

    text = open(skill_md, encoding="utf-8").read()
    fields, body, errors = parse_frontmatter(text)
    for e in errors:
        r.fail(e)

    for k in fields:
        if k not in KNOWN_KEYS:
            r.warn("unknown frontmatter key '%s' (ignored by Claude Code)" % k)

    # name
    fname = unquote(fields.get("name", ""))
    if not fname:
        r.fail("name missing")
    else:
        if fname.replace(":", "_") != name:
            r.fail("name '%s' needs folder '%s', found '%s'" % (fname, fname.replace(":", "_"), name))
        if len(fname) > MAX_NAME:
            r.fail("name longer than %d chars" % MAX_NAME)
        if not NAME_RE.match(fname):
            r.fail("name must match %s" % NAME_RE.pattern)

    # description
    desc = unquote(fields.get("description", ""))
    if not desc:
        r.fail("description missing or empty")
    else:
        if len(desc) > MAX_DESC:
            r.fail("description is %d chars (max %d)" % (len(desc), MAX_DESC))
        if XML_TAG.search(desc):
            r.fail("description contains an XML tag")
        low = desc.lower()
        if low.startswith("i ") or " i can " in low or low.startswith("you can use"):
            r.fail("description must be third person")
        if "when" not in low:
            r.warn("description never says WHEN to use the skill")
        combined = len(desc) + len(unquote(fields.get("when_to_use", "")))
        if combined > MAX_DESC_COMBINED:
            r.fail("description + when_to_use is %d chars (listing truncates at %d)" % (combined, MAX_DESC_COMBINED))

    # allowed-tools
    tools = unquote(fields.get("allowed-tools", ""))
    if tools:
        toks = [t.strip() for t in re.split(r"[,\s]+", tools) if t.strip()]
        if "Bash" in toks:
            r.warn("allowed-tools grants bare 'Bash' — scope it, e.g. Bash(go *)")

    # body size
    body_lines = body.count("\n") + (1 if body and not body.endswith("\n") else 0)
    tokens = len(body) // 4
    if body_lines > MAX_BODY_LINES:
        r.fail("body is %d lines (max %d) — move detail to a reference file" % (body_lines, MAX_BODY_LINES))
    if tokens > MAX_BODY_TOKENS:
        r.fail("body is ~%d tokens (max %d)" % (tokens, MAX_BODY_TOKENS))
    elif tokens > WARN_BODY_TOKENS:
        r.warn("body is ~%d tokens — consider moving detail to a reference file" % tokens)

    # paths and links
    for m in HARDCODED.finditer(body):
        r.fail("hardcoded skill path '%s…' — use ${CLAUDE_SKILL_DIR}" % m.group(0))
    if FORCE_LOAD.search(body):
        r.warn("'@path' force-loads a file into context — use a plain link instead")
    for m in SKILL_DIR_REF.finditer(body):
        rel = m.group(1)
        if not os.path.exists(os.path.join(path, rel)):
            r.fail("${CLAUDE_SKILL_DIR}/%s does not exist" % rel)
        before = body[max(0, m.start() - 12):m.start()].split()
        prev = before[-1].strip("`") if before else ""
        if rel.startswith("scripts/") and prev not in INTERPRETERS:
            r.warn("run scripts through an interpreter (bash/python3 …): %s" % rel)
    for m in MD_LINK.finditer(body):
        target = m.group(1)
        if target.startswith(("http://", "https://", "#", "mailto:")):
            continue
        target = target.replace("${CLAUDE_SKILL_DIR}/", "").split("#")[0]
        full = os.path.join(path, target)
        if not os.path.exists(full):
            r.fail("link target missing: %s" % target)
        elif full.endswith(".md"):
            sub = open(full, encoding="utf-8").read()
            for n in MD_LINK.finditer(sub):
                t = n.group(1)
                if not t.startswith(("http://", "https://", "#", "mailto:")) and t.endswith(".md"):
                    r.warn("%s links on to %s — keep references one level deep" % (target, t))
                    break
    return r


def list_skills(root):
    out = []
    for d in sorted(os.listdir(root)):
        p = os.path.join(root, d)
        if d.startswith(("_", ".")) or not os.path.isdir(p):
            continue
        if os.path.isfile(os.path.join(p, "SKILL.md")):
            out.append(p)
    return out


def run(paths, quiet=False):
    bad = 0
    for p in paths:
        r = check_skill(p)
        status = "FAIL" if r.fails else ("WARN" if r.warns else "OK")
        if r.fails:
            bad += 1
        if quiet and status != "FAIL":
            continue  # -q prints failures only; warnings show without -q
        print("%-4s %s" % (status, r.skill))
        for f in r.fails:
            print("       ✖ %s" % f)
        for w in r.warns:
            print("       · %s" % w)
    if bad or not quiet:
        print("%d skills checked, %d failing" % (len(paths), bad))
    return 1 if bad else 0


# --- self-test ---------------------------------------------------------------
GOOD = """---
name: %s
description: "Use when the user asks for a demo: a short recipe. Third person."
allowed-tools: Bash(${CLAUDE_SKILL_DIR}/scripts/*)
---

# Demo

Run `bash ${CLAUDE_SKILL_DIR}/scripts/go.sh`. See [notes](notes.md).
"""


def self_test():
    tmp = tempfile.mkdtemp(prefix="skillcheck-")
    try:
        def make(folder, text, extra=None):
            d = os.path.join(tmp, folder.replace(":", "_"))
            os.makedirs(os.path.join(d, "scripts"))
            open(os.path.join(d, "SKILL.md"), "w").write(text)
            open(os.path.join(d, "scripts", "go.sh"), "w").write("#!/bin/sh\n")
            open(os.path.join(d, "notes.md"), "w").write("# notes\n")
            for e in extra or []:
                os.makedirs(os.path.join(d, e), exist_ok=True)
            return d

        cases = [
            ("ps:good", GOOD % "ps:good", [], 0),
            ("ps:colon", GOOD.replace('"Use when the user asks for a demo: a short recipe. Third person."',
                                      'Use when: a short recipe') % "ps:colon", [], 1),
            ("ps:mismatch", GOOD % "ps:other", [], 1),
            ("ps:badname", GOOD % "ps:Bad_Name", [], 1),
            ("ps:hardpath", (GOOD % "ps:hardpath").replace("${CLAUDE_SKILL_DIR}/scripts/go.sh",
                                                           "~/.claude/skills/ps_hardpath/scripts/go.sh"), [], 1),
            ("ps:deadlink", (GOOD % "ps:deadlink").replace("notes.md", "nope.md"), [], 1),
            ("ps:junk", GOOD % "ps:junk", [".ruff_cache"], 1),
            ("ps:firstperson", (GOOD % "ps:firstperson").replace("Use when the user asks for a demo: a short recipe. Third person.",
                                                                 "I can help you with demos when asked."), [], 1),
            ("ps:toolong", (GOOD % "ps:toolong") + ("x" * 4 + "\n") * 600, [], 1),
        ]
        failures = 0
        for folder, text, extra, want in cases:
            d = make(folder, text, extra)
            r = check_skill(d)
            got = 1 if r.fails else 0
            mark = "ok " if got == want else "BAD"
            if got != want:
                failures += 1
            print("%s %-16s want=%d got=%d %s" % (mark, folder, want, got, r.fails[:1]))
        print("self-test: %d of %d cases wrong" % (failures, len(cases)))
        return 1 if failures else 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    args = sys.argv[1:]
    if "--self-test" in args:
        sys.exit(self_test())
    quiet = "-q" in args
    targets = [a for a in args if not a.startswith("-")]
    paths = [os.path.join(HERE, t.replace(":", "_")) if not os.path.isabs(t) else t for t in targets] or list_skills(HERE)
    sys.exit(run(paths, quiet))
