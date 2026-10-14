#!/usr/bin/env python3
"""Build agent config for other harnesses from claude/. Python 3 stdlib only.

  agent-config.py cursor --profile owner|shared [--dry-run]
  agent-config.py user-rules [--profile owner|shared] [--copy]   # for Cursor User Rules
  agent-config.py claude-settings build [--dry-run]       # ~/.claude/settings.json
  agent-config.py claude-settings promote <dotted.key>    # private layer -> repo base

`cursor` writes generated files under ~/.cursor (rules, agents) and merges keys into
the JSON files Cursor also writes itself (hooks.json, mcp.json, cli-config.json).
`cursor` never writes under ~/.claude. Design and gap table: docs/cursor-port.md.

`claude-settings build` writes ~/.claude/settings.json: the repo base
(claude/settings.json) with the private layer (~/.config/dotfiles/claude-settings.json)
on top. A change Claude made to the live file since the last build moves into the
private layer first, so nothing is lost and nothing reaches the public repo.
The owner profile adds the private ~/.config/dotfiles/owner.md to the rules.

Ownership: a generated Markdown file holds MARK. A file without it is the user's and
is left alone. JSON merges keep the user's values (except the attribution switches)
and back the file up to <file>.bak-<timestamp> before a change.
"""
import argparse
import copy
import datetime
import hashlib
import json
import os
import re
import shlex
import shutil
import stat
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CLAUDE = REPO / "claude"
CURSOR_SRC = REPO / "cursor"
GUARD = CURSOR_SRC / "hooks" / "guard_cursor.py"
GUARD_TAG = "cursor/hooks/guard_cursor.py"  # marks our hooks.json entries, quoting aside
MANAGED = ".dotfiles-managed.json"  # in ~/.cursor: the permission rules we added last run
MARK = "generated-by: dotfiles"
CONF = Path(".config") / "dotfiles"  # per-machine switches, under HOME (AGENTS.md "Profiles")
OWNER_MD = CONF / "owner.md"  # the private rules, free text; never in the repo
SETTINGS = CONF / "settings"  # NAME="value" lines (settings.example); dots settings edits it
PROFILES = ("owner", "shared")
WRITE_TOOLS = {"Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"}

# Cursor hook event -> (guard_cursor.py mode, matcher) for G3, the context ceiling.
# The CLI (2026.10.01) fires no subagentStart: a spawn shows up as preToolUse with
# tool_name "Task" (tested 2026-10-13), so both events gate it. The matcher is a regex
# on the tool name; the subagent tool name varies by model.
HOOK_EVENTS = {
    "beforeSubmitPrompt": ("prompt", None),
    "subagentStart": ("subagent", None),
    "preToolUse": ("subagent", "^(Task|Subagent|mcp_task)$"),
    "preCompact": ("compact", None),
    "afterAgentResponse": ("response", None),
}

# --- output, in the voice of scripts/setup-lib.sh ---------------------------
if sys.stdout.isatty() and not os.environ.get("NO_COLOR"):
    _G, _Y, _D, _B, _0 = "\033[38;5;108m", "\033[38;5;179m", "\033[38;5;245m", "\033[38;5;73m", "\033[0m"
else:
    _G = _Y = _D = _B = _0 = ""
DRY = False


def ok(msg):
    print(f"{_G}✓{_0} {msg}")


def info(msg):
    print(f"{_B}»»{_0} {msg}")


def skip(msg):
    print(f"{_D}•{_0} {msg} {_D}(already set up, skipping){_0}")


def warn(msg):
    print(f"{_Y}!{_0}  {msg}", file=sys.stderr)


def tilde(p):
    s = str(p)
    home = str(Path.home())
    return "~" + s[len(home):] if s == home or s.startswith(home + "/") else s


# --- text helpers -----------------------------------------------------------


def _unquote(v):
    if len(v) >= 2 and v[0] == v[-1] and v[0] in "'\"":
        if v[0] == '"':
            try:
                return json.loads(v)
            except ValueError:
                pass
        return v[1:-1].replace("''", "'") if v[0] == "'" else v[1:-1]
    return v


def split_frontmatter(text):
    """(dict of top-level keys, body). No frontmatter -> ({}, text).

    Not full YAML, but enough for agent and skill headers: `key: value` with optional
    quotes, block scalars (`|`, `>`), `- item` lists (joined with ", ", like `tools:`),
    and indented continuation lines. Comment lines are skipped.
    """
    norm = text.replace("\r\n", "\n")
    m = re.match(r"\A---\n(.*?)\n---\n?(.*)\Z", norm, re.S)
    if not m:
        return {}, text
    meta, key, style, items, block = {}, None, None, [], []

    def flush():
        if key is None:
            return
        if items:
            meta[key] = ", ".join(items)
        elif style == "|":
            meta[key] = "\n".join(block).strip("\n")
        elif block:
            meta[key] = " ".join(b for b in (x.strip() for x in block) if b)

    for line in m.group(1).splitlines():
        if line.lstrip().startswith("#") and style is None:
            continue
        km = re.match(r"([A-Za-z0-9_-]+):(?:\s+(.*)|\s*)\Z", line)
        if km:
            flush()
            key, items, block = km.group(1), [], []
            value = (km.group(2) or "").strip()
            style = value[0] if value[:1] in ("|", ">") else None
            if style is None:
                meta[key] = _unquote(value)
                if value:
                    block = [meta[key]]
            continue
        if key is None:
            continue
        lm = re.match(r"\s*-\s+(.*)\Z", line)
        if lm and style is None and meta.get(key) == "":
            items.append(_unquote(lm.group(1).strip()))
        elif line[:1] in (" ", "\t") or (style and not line.strip()):
            if style == "|":
                block.append(line.strip())
            else:
                block.append(line)
    flush()
    return meta, m.group(2)


def owner_overlay(home, profile):
    """The owner's private rules file, or None (shared profile, or no file)."""
    f = home / OWNER_MD
    return f if profile == "owner" and f.is_file() else None


def rules_text(home, profile):
    """claude/CLAUDE.md, plus the owner overlay on the owner profile."""
    text = (CLAUDE / "CLAUDE.md").read_text()
    extra = owner_overlay(home, profile)
    return text.rstrip("\n") + "\n\n" + extra.read_text() if extra else text


def yaml_str(v):
    """A one-line YAML value: plain when safe, else a JSON string (JSON is valid YAML)."""
    if "\n" in v or ": " in v or " #" in v or v[:1] in "!&*?|>%@`'\"[]{},#-" or v.strip() != v:
        return json.dumps(v, ensure_ascii=False)
    return v


def mdc(description, body):
    return f"---\ndescription: {description}\nalwaysApply: true\n---\n{body}"


def stamp(*srcs):
    edit = "that file" if len(srcs) == 1 else "those files"
    return f"<!-- {MARK} (scripts/agent-config.py) from {' and '.join(srcs)}. Edit {edit}, then run dots. -->\n\n"


# --- file writers -----------------------------------------------------------


def atomic_write(path, text, mode=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as f:
            f.write(text)
        if mode is not None:
            os.chmod(tmp, mode)
        elif path.exists():
            os.chmod(tmp, path.stat().st_mode & 0o777)
        else:
            os.chmod(tmp, 0o644)
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def write_generated(path, text, mode=None):
    """Write a generated Markdown file unless a user-owned file is in the way.
    mode: the file mode to set; None keeps the old one (0644 for a new file)."""
    if path.is_symlink() or (path.exists() and MARK not in path.read_text(errors="replace")):
        warn(f"{tilde(path)} exists and is not ours, left untouched")
        return
    if path.exists() and path.read_text() == text:
        if mode is None or stat.S_IMODE(path.stat().st_mode) == mode:
            skip(tilde(path))
            return
    if DRY:
        info(f"would write {tilde(path)}")
        return
    atomic_write(path, text, mode)
    ok(f"wrote {tilde(path)}")


def remove_generated(path, why="its source is gone"):
    if path.is_file() and not path.is_symlink() and MARK in path.read_text(errors="replace"):
        if DRY:
            info(f"would remove {tilde(path)}")
            return
        path.unlink()
        ok(f"removed {tilde(path)} ({why})")


def load_json(path):
    """Parsed JSON object, {} if the file is missing, None if it is unreadable."""
    if not path.exists():
        return {}
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def save_json(path, old, new, private=False):
    if new == old:
        skip(tilde(path))
        return
    if path.is_symlink():
        warn(f"{tilde(path)} is a symlink (yours), left untouched")
        return
    if DRY:
        info(f"would update {tilde(path)}")
        return
    if path.exists():
        bak = path.with_name(f"{path.name}.bak-{datetime.datetime.now():%Y%m%d-%H%M%S}")
        atomic_write(bak, path.read_text(), path.stat().st_mode & 0o777)
    atomic_write(path, json.dumps(new, indent=2) + "\n", 0o600 if private else None)
    ok(f"updated {tilde(path)}")


# --- Cursor parts -----------------------------------------------------------


CURSOR_NOTE = (
    "> In Cursor, read \"Claude\" as \"the agent\". Claude Code tool names map to Cursor:\n"
    "> Agent = Task (subagent), AskUserQuestion = AskQuestion, `/ps:*` skills work the same.\n"
    "> Rules about Claude-only features apply where Cursor has an equivalent.\n\n"
)
# Present when the user pasted the rules into Cursor Settings > Rules > User Rules
# (`agent-config.py user-rules --copy`). Holds the hash of the pasted text.
USER_RULES_MARK = CONF / "cursor-user-rules.sha256"


def output_style():
    """(name, body) of the active Claude output style, or None."""
    style = (load_json(CLAUDE / "settings.json") or {}).get("outputStyle")
    if not isinstance(style, str) or style.lower() == "default":
        return None
    for f in sorted((CLAUDE / "output-styles").glob("*.md")):
        meta, body = split_frontmatter(f.read_text())
        if f.stem == style or meta.get("name") == style:
            return style, f, body.lstrip()
    return None


def user_rules_text(home, profile):
    text = CURSOR_NOTE + rules_text(home, profile)
    style = output_style()
    if style:
        text = text.rstrip("\n") + f"\n\n# Reply style: {style[0]}\n\n" + style[2]
    return text


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def cursor_rules(home, profile):
    rules = home / ".cursor" / "rules"
    mark = home / USER_RULES_MARK
    if mark.exists():  # the rules live in Cursor's User Rules: no file copy (no duplicates)
        for f in ("claude-global.mdc", "claude-output-style.mdc"):
            remove_generated(rules / f, "the rules are in Cursor User Rules")
        if mark.read_text().strip() == sha(user_rules_text(home, profile)):
            skip("Cursor User Rules (pasted in Cursor Settings)")
        else:
            warn("the rules changed. Run cursor-rules, then paste in Cursor Settings > Rules > User Rules")
        return
    srcs, desc, mode = ["claude/CLAUDE.md"], "Global rules from the dotfiles repo (claude/CLAUDE.md)", None
    if owner_overlay(home, profile):
        # owner.md is private (0600), so its copy here is private too.
        srcs.append(f"~/{OWNER_MD}")
        desc, mode = desc + " and the owner's private rules", 0o600
    elif profile == "owner":
        warn(f"no ~/{OWNER_MD}, so the Cursor rule has no owner-only rules. Add them with: dots settings")
    body = stamp(*srcs) + CURSOR_NOTE + rules_text(home, profile)
    write_generated(rules / "claude-global.mdc", mdc(desc, body), mode)

    target = rules / "claude-output-style.mdc"
    style = output_style()
    if style is None:
        remove_generated(target)
        return
    name, style_file, sbody = style
    body = stamp(f"claude/output-styles/{style_file.name}") + f"# Reply style: {name}\n\n" + sbody
    write_generated(target, mdc(f'Reply style "{name}" (Claude outputStyle)', body))


def cursor_agents(home):
    dst = home / ".cursor" / "agents"
    wanted = set()
    for src in sorted((CLAUDE / "agents").glob("*.md")):
        if src.name == "README.md":
            continue
        meta, body = split_frontmatter(src.read_text())
        name = meta.get("name") or src.stem
        lines = [f"name: {name}"]
        if meta.get("description"):
            lines.append(f"description: {yaml_str(meta['description'])}")
        lines.append("model: inherit")  # Claude aliases like `sonnet` are not Cursor model IDs
        tools = {t.strip() for t in meta.get("tools", "").split(",") if t.strip()}
        if tools and not tools & WRITE_TOOLS:
            lines.append("readonly: true")  # Cursor does not enforce `tools:`
        text = "---\n" + "\n".join(lines) + "\n---\n" + stamp(f"claude/agents/{src.name}") + body.lstrip()
        out = dst / f"{name}.md"
        wanted.add(out.name)
        write_generated(out, text)
    if dst.is_dir():
        for f in sorted(dst.glob("*.md")):
            if f.name not in wanted:
                remove_generated(f)


def cursor_hooks(home):
    path = home / ".cursor" / "hooks.json"
    old = load_json(path)
    if old is None:
        warn(f"{tilde(path)} is not valid JSON, left untouched")
        return
    new = json.loads(json.dumps(old))
    new.setdefault("version", 1)
    hooks = new.setdefault("hooks", {})
    if not isinstance(hooks, dict):
        warn(f"{tilde(path)} has a non-object `hooks`, left untouched")
        return
    slot = {}  # event -> index of our old entry, so a rerun keeps it in the same place
    for event in list(hooks):  # drop our old entries, keep everyone else's
        entries = hooks[event] if isinstance(hooks[event], list) else []
        ours = [i for i, e in enumerate(entries) if isinstance(e, dict) and GUARD_TAG in str(e.get("command", ""))]
        if ours:
            slot[event] = ours[0]
            hooks[event] = [e for i, e in enumerate(entries) if i not in ours]
        if not hooks[event]:
            del hooks[event]
    if GUARD.is_file():
        q = shlex.quote(str(GUARD))
        for event, (mode, matcher) in HOOK_EVENTS.items():
            # Cursor runs this with /bin/sh -c. A missing script must not exit 2 (= deny).
            entry = {"command": f"test -r {q} || exit 0; python3 {q} {mode}", "timeout": 10}
            if matcher:
                entry["matcher"] = matcher
            entries = hooks.setdefault(event, [])
            entries.insert(slot.get(event, len(entries)), entry)
    save_json(path, old, new)


def setting(home, name):
    """The value of NAME in ~/.config/dotfiles/settings, or "".

    Same rules as setting in scripts/setup-lib.sh: the last NAME line wins, and one
    pair of "" around the value goes.
    """
    try:
        data = (home / SETTINGS).read_bytes()
    except OSError:
        return ""
    # Lines end at "\n" only, and odd bytes do not stop the read, as in awk.
    lines = data.decode("utf-8", "surrogateescape").split("\n")
    value = ""
    for line in lines:
        if line.startswith(name + "="):
            value = line[len(name) + 1:].rstrip().removeprefix('"').removesuffix('"')
    return value


def mcp_wanted(home):
    """MCP servers are opt-in on every profile: connectors are never forced on.

    DOTFILES_MCP=on|off wins, then CURSOR_MCP in the settings (dots settings).
    """
    v = os.environ.get("DOTFILES_MCP", "").strip().lower() or setting(home, "CURSOR_MCP").strip().lower()
    return v in ("1", "on", "yes", "true")


def cursor_mcp(home):
    path = home / ".cursor" / "mcp.json"
    if not mcp_wanted(home):
        info("MCP servers from cursor/mcp.json are opt-in: turn on Cursor MCP in dots settings")
        return
    ours = (load_json(CURSOR_SRC / "mcp.json") or {}).get("mcpServers") or {}
    old = load_json(path)
    if old is None:
        warn(f"{tilde(path)} is not valid JSON, left untouched")
        return
    new = json.loads(json.dumps(old))
    servers = new.setdefault("mcpServers", {})
    for name, cfg in ours.items():
        servers.setdefault(name, cfg)  # the user's entry of the same name wins
    save_json(path, old, new)


def translate_permission(rule, kind="allow"):
    """Claude permission rule -> Cursor CLI rule, or None when Cursor has no match."""
    m = re.fullmatch(r"(\w+)\((.*)\)", rule.strip())
    if not m:
        return None
    tool, arg = m.groups()
    if tool == "Bash":
        if re.search(r"(:\*|\s\*)\Z", arg):  # prefix rule: Shell(x) matches x and "x ..."
            cmd = re.sub(r"(:\*|\s\*)\Z", "", arg).strip()
            return f"Shell({cmd})" if cmd and "*" not in cmd else None
        # Claude matches this one exactly. The CLI's exact form is Shell(<exe>:<args>);
        # a bare word has no exact form, so it is skipped (narrower is safer).
        exe, _, rest = arg.strip().partition(" ")
        return f"Shell({exe}:{rest.strip()})" if exe and rest.strip() and "*" not in arg else None
    if tool in ("Read", "Edit", "Write"):
        path = arg[2:] if arg.startswith("./") else arg
        if kind == "deny" and not path.startswith(("/", "~", "**")):
            path = "**/" + path  # the CLI never matches a bare relative deny (tested 2026-10-13)
        return f"{'Read' if tool == 'Read' else 'Write'}({path})"
    if tool == "WebFetch" and arg.startswith("domain:"):
        return f"WebFetch({arg[len('domain:'):]})"
    return None


def cursor_cli_config(home):
    path = home / ".cursor" / "cli-config.json"
    old = load_json(path)
    if old is None:
        warn(f"{tilde(path)} is not valid JSON, left untouched")
        return
    new = json.loads(json.dumps(old))
    new.setdefault("version", 1)
    new.setdefault("editor", {"vimMode": False})
    perms = new.setdefault("permissions", {})
    src = (load_json(CLAUDE / "settings.json") or {}).get("permissions") or {}
    side = home / ".cursor" / MANAGED
    managed = load_json(side)
    managed = managed if isinstance(managed, dict) else {}
    added = {}
    for kind in ("allow", "deny"):
        have = perms.setdefault(kind, [])
        rules = src.get(kind) or []
        want = [m for m in (translate_permission(r, kind) for r in rules) if m]
        prev = set(managed.get(kind) or [])
        have[:] = [r for r in have if r in want or r not in prev]  # drop our stale rules
        for m in want:
            if m not in have:
                have.append(m)
        added[kind] = want
    # Shared rule "never add the agent as co-author": the CLI defaults these to true.
    attr = new.setdefault("attribution", {})
    attr["attributeCommitsToAgent"] = False
    attr["attributePRsToAgent"] = False
    statusline = home / ".claude" / "statusline.sh"
    if "statusLine" not in new and statusline.exists():
        new["statusLine"] = {"type": "command", "command": str(statusline), "padding": 0}
    save_json(path, old, new, private=True)  # the CLI keeps login data in this file
    if not DRY and load_json(path) == new and managed != added:
        atomic_write(side, json.dumps(added, indent=2) + "\n")


def cmd_cursor(args):
    home = Path.home()
    info(f"Cursor config from {tilde(CLAUDE)} (profile: {args.profile})")
    failed = 0
    for part in (lambda: cursor_rules(home, args.profile), lambda: cursor_agents(home),
                 lambda: cursor_hooks(home), lambda: cursor_mcp(home), lambda: cursor_cli_config(home)):
        try:
            part()
        except Exception as e:  # one bad file must not stop the other parts
            warn(f"skipped a part: {type(e).__name__}: {e}")
            failed += 1
    return 1 if failed else 0


def clipboard_command():
    """The command that copies stdin to the clipboard: pbcopy on a Mac, else wl-copy, xclip or xsel."""
    if sys.platform == "darwin":
        return ["pbcopy"]
    for cmd in (["wl-copy"], ["xclip", "-selection", "clipboard"], ["xsel", "--clipboard", "--input"]):
        if shutil.which(cmd[0]):
            return cmd
    return None


def cmd_user_rules(args):
    """Print the rules for Cursor's User Rules box, or copy them and switch off the file copy.

    Cursor keeps User Rules on the account, so no file can fill them. They apply in every
    folder; the ~/.cursor/rules file only applies to folders under the home folder.
    """
    home = Path.home()
    text = user_rules_text(home, args.profile)
    if not args.copy:
        sys.stdout.write(text)
        return 0
    import subprocess

    copy = clipboard_command()
    if not copy:
        warn("no clipboard tool found (install wl-clipboard or xclip). Printing the rules instead.")
        sys.stdout.write(text)
        return 1
    subprocess.run(copy, input=text.encode(), check=True)
    atomic_write(home / USER_RULES_MARK, sha(text) + "\n")
    for f in ("claude-global.mdc", "claude-output-style.mdc"):
        remove_generated(home / ".cursor" / "rules" / f, "the rules move to Cursor User Rules")
    ok("copied the rules to the clipboard")
    info("Now: Cursor > Cursor Settings > Rules > User Rules > paste > save.")
    info(f"To go back to the file copy: rm {tilde(home / USER_RULES_MARK)} && dots")
    return 0


# --- Claude settings: repo base + private layer -> ~/.claude/settings.json ----
LAYER = CONF / "claude-settings.json"  # private, never in the repo; any profile
LAST = CONF / "claude-settings.last.json"  # what the last build wrote, to see Claude's changes
LIVE = Path(".claude") / "settings.json"
# Lists that merge as a union (base first). Every other list is replaced whole.
UNION = {("permissions", "allow"), ("permissions", "deny"), ("permissions", "ask"),
         ("permissions", "additionalDirectories")}


def merge(base, layer, path=()):
    """base with layer on top. A None in the layer removes the key."""
    out = copy.deepcopy(base)
    for k, v in layer.items():
        p = path + (k,)
        if v is None:
            out.pop(k, None)
        elif isinstance(out.get(k), dict) and isinstance(v, dict):
            out[k] = merge(out[k], v, p)
        elif p in UNION and isinstance(out.get(k), list) and isinstance(v, list):
            out[k] = out[k] + [x for x in v if x not in out[k]]
        else:
            out[k] = copy.deepcopy(v)
    return out


def drift(last, live, path=()):
    """What changed from last to live, as a layer patch, plus notes (kind, path, n)."""
    patch, notes = {}, []
    for k, v in live.items():
        p = path + (k,)
        old = last.get(k)
        if k not in last:
            patch[k] = copy.deepcopy(v)
            notes.append(("set", p, 0))
        elif isinstance(old, dict) and isinstance(v, dict):
            sub, n = drift(old, v, p)
            if sub:
                patch[k] = sub
            notes += n
        elif p in UNION and isinstance(old, list) and isinstance(v, list):
            added = [x for x in v if x not in old]
            removed = [x for x in old if x not in v]
            if added:
                patch[k] = added
                notes.append(("add", p, len(added)))
            if removed:
                notes.append(("removed", p, len(removed)))
        elif old != v:
            patch[k] = copy.deepcopy(v)
            notes.append(("set", p, 0))
    for k in last:
        if k not in live:
            patch[k] = None
            notes.append(("unset", path + (k,), 0))
    return patch, notes


def apply_patch(layer, patch, path=()):
    """Store a drift patch in the layer. None stays as a removal."""
    out = copy.deepcopy(layer)
    for k, v in patch.items():
        p = path + (k,)
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = apply_patch(out[k], v, p)
        elif p in UNION and isinstance(v, list) and isinstance(out.get(k), list):
            out[k] = out[k] + [x for x in v if x not in out[k]]
        else:
            out[k] = copy.deepcopy(v)
    return out


def dump(obj):
    return json.dumps(obj, indent=2, ensure_ascii=False) + "\n"


def claude_settings_build(home):
    base_f, layer_f, last_f, live_f = CLAUDE / "settings.json", home / LAYER, home / LAST, home / LIVE
    base = load_json(base_f)
    layer = load_json(layer_f)
    if base is None or layer is None:
        bad = base_f if base is None else layer_f
        warn(f"{tilde(bad)} is not valid JSON. Fix it; ~/.claude/settings.json left as it is.")
        return 1
    was_link = live_f.is_symlink()
    notes = []
    if live_f.exists() and not was_link:
        live = load_json(live_f)
        if live is None:
            warn(f"{tilde(live_f)} is not valid JSON (Claude may be writing it). Run dots again.")
            return 1
        last = load_json(last_f) if last_f.exists() else None
        if last is None:  # no record of the last build: compare with what it would be
            last = merge(base, layer)
        patch, notes = drift(last, live)
        if patch:
            layer = apply_patch(layer, patch)
            if DRY:
                info(f"would keep {len(notes)} change(s) in {tilde(layer_f)}")
            else:
                atomic_write(layer_f, dump(layer), 0o600)
    for kind, p, n in notes:
        key = ".".join(p)
        if kind == "removed":
            warn(f"Claude removed {n} item(s) from {key}, but the repo or your private layer has them. "
                 f"Remove them there to make it stick.")
        else:
            what = {"set": "changed", "add": f"+{n} item(s)", "unset": "removed"}[kind]
            info(f"kept in your private layer: {key} ({what})")
    text = dump(merge(base, layer))
    if not was_link and live_f.exists() and live_f.read_text() == text:
        skip(tilde(live_f))
    elif DRY:
        info(f"would write {tilde(live_f)}")
        return 0
    else:
        if was_link:
            info(f"{tilde(live_f)} was a link into the repo; it is now a built file")
            live_f.unlink()
        atomic_write(live_f, text, 0o644)
        ok(f"built {tilde(live_f)} (repo base + private layer)")
    if not DRY and (not last_f.exists() or last_f.read_text() != text):
        atomic_write(last_f, text, 0o600)
    return 0


def get_path(obj, path):
    for k in path:
        if not isinstance(obj, dict) or k not in obj:
            raise KeyError(".".join(path))
        obj = obj[k]
    return obj


def drop_path(obj, path):
    """Remove path from obj and prune the dicts it leaves empty."""
    if len(path) == 1:
        obj.pop(path[0], None)
        return
    sub = obj.get(path[0])
    if isinstance(sub, dict):
        drop_path(sub, path[1:])
        if not sub:
            obj.pop(path[0])


def claude_settings_promote(home, dotted):
    path = tuple(dotted.split("."))
    base_f, layer_f = CLAUDE / "settings.json", home / LAYER
    base, layer = load_json(base_f), load_json(layer_f)
    if base is None or layer is None:
        warn("the repo base or the private layer is not valid JSON; nothing moved")
        return 1
    try:
        value = get_path(layer, path)
    except KeyError:
        warn(f"{dotted} is not in {tilde(layer_f)}")
        return 1
    patch = value
    for k in reversed(path):
        patch = {k: patch}
    new_base = merge(base, patch)
    drop_path(layer, path)
    if DRY:
        info(f"would move {dotted} to {tilde(base_f)}")
        return 0
    atomic_write(base_f, dump(new_base))
    atomic_write(layer_f, dump(layer), 0o600)
    ok(f"moved {dotted} to {tilde(base_f)}. Commit it to share it.")
    return claude_settings_build(home)


def cmd_claude_settings(args):
    home = Path.home()
    if args.action == "promote":
        if not args.key:
            warn("promote needs a key, e.g. permissions.allow")
            return 2
        return claude_settings_promote(home, args.key)
    return claude_settings_build(home)


def default_profile():
    """Same order as dotfiles_profile in scripts/setup-lib.sh (which also saves a guess)."""
    home = Path.home()
    p = os.environ.get("DOTFILES_PROFILE", "") or setting(home, "PROFILE")
    if not p and (home / OWNER_MD).is_file():
        p = "owner"
    return p if p in PROFILES else "shared"


def main(argv=None):
    global DRY
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("cursor", help="generate and merge Cursor config under ~/.cursor")
    c.add_argument("--profile", choices=PROFILES, default="shared")
    c.add_argument("--dry-run", action="store_true")
    u = sub.add_parser("user-rules", help="text for Cursor Settings > Rules > User Rules")
    u.add_argument("--profile", choices=PROFILES, default=default_profile())
    u.add_argument("--copy", action="store_true", help="copy to the clipboard and drop the file copy")
    u.add_argument("--dry-run", action="store_true")
    cs = sub.add_parser("claude-settings", help="build ~/.claude/settings.json from the repo base and the private layer")
    cs.add_argument("action", choices=("build", "promote"))
    cs.add_argument("key", nargs="?", help="promote: the dotted key to move to the repo, e.g. permissions.allow")
    cs.add_argument("--dry-run", action="store_true")
    args = p.parse_args(argv)
    DRY = args.dry_run
    return {"cursor": cmd_cursor, "user-rules": cmd_user_rules,
            "claude-settings": cmd_claude_settings}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
