#!/usr/bin/env python3
"""Tests for agent-config.py. Run: python3 scripts/test_agent_config.py

Every test gets its own temp HOME and its own fake repo (claude/, cursor/). The module
reads HOME through Path.home() and the repo through module globals, so both are patched
per test. Nothing here touches the real ~/.cursor or ~/.claude.
"""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "agent-config.py"

# Plain output, so message assertions do not depend on a tty.
os.environ["NO_COLOR"] = "1"
_spec = importlib.util.spec_from_file_location("agent_config", str(SCRIPT))
AC = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(AC)

CLAUDE_MD = (
    "# Global\n\nintro line\n\n"
    "## Environment\n\n- one\n- two\n\n"
    "## Working rules\n\n1. a\n2. b\n"
)
OWNER_MD = "## Off-limits\n\n- never do the secret thing\n"


def call(fn, *args, **kwargs):
    """Run fn, return (result, stdout, stderr)."""
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        result = fn(*args, **kwargs)
    return result, out.getvalue(), err.getvalue()


def wj(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj))


def rj(path):
    return json.loads(path.read_text())


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="agent-config-test-"))
        self.addCleanup(shutil.rmtree, str(self.tmp), True)
        self.home = self.tmp / "home"
        self.repo = self.tmp / "repo"
        self.home.mkdir()
        env = mock.patch.dict(os.environ, {"HOME": str(self.home)})
        env.start()
        self.addCleanup(env.stop)
        os.environ.pop("DOTFILES_PROFILE", None)
        os.environ.pop("DOTFILES_MCP", None)
        self.claude = self.repo / "claude"
        self.cursor_src = self.repo / "cursor"
        self.guard = self.cursor_src / "hooks" / "guard_cursor.py"
        for name, val in (("REPO", self.repo), ("CLAUDE", self.claude), ("CURSOR_SRC", self.cursor_src),
                          ("GUARD", self.guard), ("DRY", False)):
            p = mock.patch.object(AC, name, val)
            p.start()
            self.addCleanup(p.stop)
        self.claude.mkdir(parents=True)
        (self.claude / "CLAUDE.md").write_text(CLAUDE_MD)
        self.guard.parent.mkdir(parents=True)
        self.guard.write_text("# fake guard\n")
        self.assertEqual(str(Path.home()), str(self.home))  # never the real HOME

    # fixture helpers
    def overlay(self, text=OWNER_MD):
        f = self.home / AC.OWNER_MD
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(text)
        return f

    def settings(self, **kw):
        wj(self.claude / "settings.json", kw)

    def perms(self, allow=None, deny=None):
        self.settings(permissions={"allow": allow or [], "deny": deny or []})

    @property
    def cdir(self):
        return self.home / ".cursor"


# --- pure text helpers --------------------------------------------------------


class TranslatePermissionTest(unittest.TestCase):
    t = staticmethod(AC.translate_permission)

    def test_bash_prefix_forms(self):
        self.assertEqual(self.t("Bash(git status:*)"), "Shell(git status)")
        self.assertEqual(self.t("Bash(npm run *)"), "Shell(npm run)")
        self.assertEqual(self.t("Bash(ls:*)"), "Shell(ls)")
        self.assertEqual(self.t("Bash(rm -rf *)", "deny"), "Shell(rm -rf)")

    def test_bash_prefix_edge_cases_are_skipped(self):
        self.assertIsNone(self.t("Bash(*)"))
        self.assertIsNone(self.t("Bash(:*)"))
        self.assertIsNone(self.t("Bash(a* b:*)"))  # star inside the command part

    def test_bash_exact_form(self):
        self.assertEqual(self.t("Bash(git status)"), "Shell(git:status)")
        self.assertEqual(self.t("Bash(git push origin main)", "deny"), "Shell(git:push origin main)")

    def test_bash_exact_without_args_or_with_star_is_skipped(self):
        self.assertIsNone(self.t("Bash(ls)"))
        self.assertIsNone(self.t("Bash(git * foo)"))

    def test_read_edit_write_allow_keeps_relative_path(self):
        self.assertEqual(self.t("Read(./src/**)"), "Read(src/**)")
        self.assertEqual(self.t("Read(.env)"), "Read(.env)")
        self.assertEqual(self.t("Read(/etc/hosts)"), "Read(/etc/hosts)")

    def test_edit_and_write_both_become_write(self):
        self.assertEqual(self.t("Edit(./docs/**)"), "Write(docs/**)")
        self.assertEqual(self.t("Write(notes.md)"), "Write(notes.md)")

    def test_deny_relative_path_gets_double_star_prefix(self):
        self.assertEqual(self.t("Read(./.env)", "deny"), "Read(**/.env)")
        self.assertEqual(self.t("Read(.env)", "deny"), "Read(**/.env)")
        self.assertEqual(self.t("Edit(secrets/**)", "deny"), "Write(**/secrets/**)")

    def test_deny_absolute_home_and_glob_paths_unchanged(self):
        self.assertEqual(self.t("Read(/etc/x)", "deny"), "Read(/etc/x)")
        self.assertEqual(self.t("Read(~/.ssh/**)", "deny"), "Read(~/.ssh/**)")
        self.assertEqual(self.t("Read(**/.env)", "deny"), "Read(**/.env)")

    def test_webfetch_domain(self):
        self.assertEqual(self.t("WebFetch(domain:example.com)"), "WebFetch(example.com)")
        self.assertIsNone(self.t("WebFetch(https://example.com)"))

    def test_unknown_or_malformed_rules(self):
        for rule in ("WebSearch(x)", "Task(foo)", "mcp__srv__tool", "Read", "", "Bash", "Bash(unclosed"):
            self.assertIsNone(self.t(rule), rule)

    def test_surrounding_whitespace_is_ignored(self):
        self.assertEqual(self.t("  Bash(git status:*)  "), "Shell(git status)")


class SplitFrontmatterTest(unittest.TestCase):
    def test_basic(self):
        meta, body = AC.split_frontmatter("---\nname: foo\ndescription: does things\n---\nbody here\n")
        self.assertEqual(meta, {"name": "foo", "description": "does things"})
        self.assertEqual(body, "body here\n")

    def test_no_frontmatter(self):
        self.assertEqual(AC.split_frontmatter("just text\n"), ({}, "just text\n"))

    def test_value_containing_colon_and_dashes_in_key(self):
        meta, _ = AC.split_frontmatter("---\nallowed-tools: Read, Grep\ndescription: a: b\n---\nx")
        self.assertEqual(meta["allowed-tools"], "Read, Grep")
        self.assertEqual(meta["description"], "a: b")

    def test_comments_skipped_and_indented_line_continues_value(self):
        meta, _ = AC.split_frontmatter("---\nname: x\n  more words\n- stray item\n# note\n---\nb")
        self.assertEqual(meta, {"name": "x more words"})

    def test_quoted_values(self):
        meta, _ = AC.split_frontmatter("---\na: \"x: y\"\nb: 'it''s'\nc: \"tab\\tend\"\n---\nb")
        self.assertEqual(meta, {"a": "x: y", "b": "it's", "c": "tab\tend"})

    def test_folded_and_literal_block_scalars(self):
        text = "---\nd: >\n  one\n  two\nl: |-\n  first\n  second\nn: z\n---\nb"
        meta, _ = AC.split_frontmatter(text)
        self.assertEqual(meta, {"d": "one two", "l": "first\nsecond", "n": "z"})

    def test_list_under_empty_key_is_joined(self):
        meta, _ = AC.split_frontmatter("---\ntools:\n  - Read\n  - \"Grep\"\nname: r\n---\nb")
        self.assertEqual(meta, {"tools": "Read, Grep", "name": "r"})

    def test_crlf_frontmatter(self):
        meta, body = AC.split_frontmatter("---\r\nname: w\r\n---\r\nbody\r\n")
        self.assertEqual(meta, {"name": "w"})
        self.assertEqual(body, "body\n")

    def test_closing_fence_without_trailing_newline(self):
        self.assertEqual(AC.split_frontmatter("---\na: b\n---"), ({"a": "b"}, ""))

    def test_unterminated_frontmatter_is_plain_text(self):
        text = "---\na: b\nno closing fence\n"
        self.assertEqual(AC.split_frontmatter(text), ({}, text))

    def test_empty_value(self):
        meta, _ = AC.split_frontmatter("---\ntools:\n---\nb")
        self.assertEqual(meta, {"tools": ""})


class SmallHelpersTest(Base):
    def test_tilde(self):
        self.assertEqual(AC.tilde(self.home / ".cursor" / "x"), "~/.cursor/x")
        self.assertEqual(AC.tilde(self.home), "~")
        self.assertEqual(AC.tilde("/elsewhere/x"), "/elsewhere/x")
        self.assertEqual(AC.tilde(str(self.home) + "-other/x"), str(self.home) + "-other/x")

    def test_mdc_and_stamp(self):
        self.assertEqual(AC.mdc("d", "B"), "---\ndescription: d\nalwaysApply: true\n---\nB")
        self.assertIn(AC.MARK, AC.stamp("claude/CLAUDE.md"))

    def test_atomic_write_creates_parents_and_sets_mode(self):
        p = self.tmp / "a" / "b" / "f.txt"
        AC.atomic_write(p, "hi", 0o600)
        self.assertEqual(p.read_text(), "hi")
        self.assertEqual(stat.S_IMODE(p.stat().st_mode), 0o600)
        AC.atomic_write(p, "again")  # keeps the existing mode
        self.assertEqual(stat.S_IMODE(p.stat().st_mode), 0o600)
        self.assertEqual([x.name for x in p.parent.iterdir()], ["f.txt"])  # no temp leftovers

    def test_atomic_write_default_mode_for_new_file(self):
        p = self.tmp / "n.txt"
        AC.atomic_write(p, "x")
        self.assertEqual(stat.S_IMODE(p.stat().st_mode), 0o644)

    def test_load_json(self):
        p = self.tmp / "j.json"
        self.assertEqual(AC.load_json(p), {})  # missing
        p.write_text("{bad")
        self.assertIsNone(AC.load_json(p))
        p.write_text("[1, 2]")
        self.assertIsNone(AC.load_json(p))  # not an object
        p.write_text('{"a": 1}')
        self.assertEqual(AC.load_json(p), {"a": 1})

    def test_default_profile_order(self):
        self.assertEqual(AC.default_profile(), "shared")
        pf = self.home / ".config" / "dotfiles" / "profile"
        pf.parent.mkdir(parents=True)
        pf.write_text("owner\n")
        self.assertEqual(AC.default_profile(), "owner")
        os.environ["DOTFILES_PROFILE"] = "shared"
        self.assertEqual(AC.default_profile(), "shared")  # env beats the file
        os.environ["DOTFILES_PROFILE"] = "bogus"
        self.assertEqual(AC.default_profile(), "shared")  # invalid -> default

    def test_default_profile_owner_when_overlay_exists(self):
        f = self.home / AC.OWNER_MD
        f.parent.mkdir(parents=True)
        f.write_text("x\n")
        self.assertEqual(AC.default_profile(), "owner")
        (self.home / AC.CONF / "profile").write_text("shared\n")
        self.assertEqual(AC.default_profile(), "shared")  # the file beats the overlay
        os.environ["DOTFILES_PROFILE"] = "shared"
        self.assertEqual(AC.default_profile(), "shared")

    def test_default_profile_garbage_in_file(self):
        pf = self.home / ".config" / "dotfiles" / "profile"
        pf.parent.mkdir(parents=True)
        pf.write_text("admin\n")
        self.assertEqual(AC.default_profile(), "shared")


# --- profiles -----------------------------------------------------------------


class OwnerOverlayTest(Base):
    def test_owner_gets_source_then_overlay(self):
        self.overlay()
        self.assertEqual(AC.rules_text(self.home, "owner"), CLAUDE_MD.rstrip("\n") + "\n\n" + OWNER_MD)

    def test_owner_without_overlay_gets_the_source(self):
        self.assertEqual(AC.rules_text(self.home, "owner"), CLAUDE_MD)
        self.assertIsNone(AC.owner_overlay(self.home, "owner"))

    def test_shared_never_reads_the_overlay(self):
        self.overlay()
        self.assertEqual(AC.rules_text(self.home, "shared"), CLAUDE_MD)
        self.assertIsNone(AC.owner_overlay(self.home, "shared"))

    def test_overlay_that_is_a_directory_is_ignored(self):
        (self.home / AC.OWNER_MD).mkdir(parents=True)
        self.assertEqual(AC.rules_text(self.home, "owner"), CLAUDE_MD)

    def test_user_rules_text_has_cursor_note_first(self):
        t = AC.user_rules_text(self.home, "owner")
        self.assertTrue(t.startswith(AC.CURSOR_NOTE))
        self.assertTrue(t.endswith(CLAUDE_MD))

    def test_user_rules_text_appends_output_style_after_the_overlay(self):
        self.overlay()
        self.settings(outputStyle="Terse")
        (self.claude / "output-styles").mkdir()
        (self.claude / "output-styles" / "terse.md").write_text("---\nname: Terse\n---\n\nBe brief.\n")
        t = AC.user_rules_text(self.home, "shared")
        self.assertIn("# Reply style: Terse\n\nBe brief.\n", t)
        self.assertNotIn("Off-limits", t)
        t = AC.user_rules_text(self.home, "owner")
        self.assertLess(t.index("## Off-limits"), t.index("# Reply style: Terse"))


# --- generated Markdown ownership ---------------------------------------------


class WriteGeneratedTest(Base):
    def setUp(self):
        super().setUp()
        self.f = self.cdir / "rules" / "x.md"
        self.text = "<!-- %s -->\nhello\n" % AC.MARK

    def test_creates_file_and_parents(self):
        _, out, _ = call(AC.write_generated, self.f, self.text)
        self.assertEqual(self.f.read_text(), self.text)
        self.assertIn("wrote ~/.cursor/rules/x.md", out)

    def test_rewrites_our_own_file_when_text_changes(self):
        call(AC.write_generated, self.f, self.text)
        new = self.text + "more\n"
        call(AC.write_generated, self.f, new)
        self.assertEqual(self.f.read_text(), new)

    def test_unchanged_text_is_skipped(self):
        call(AC.write_generated, self.f, self.text)
        before = self.f.stat().st_mtime_ns
        _, out, _ = call(AC.write_generated, self.f, self.text)
        self.assertIn("already set up", out)
        self.assertEqual(self.f.stat().st_mtime_ns, before)

    def test_user_file_without_marker_is_left_alone(self):
        self.f.parent.mkdir(parents=True)
        self.f.write_text("my own rules\n")
        _, out, err = call(AC.write_generated, self.f, self.text)
        self.assertEqual(self.f.read_text(), "my own rules\n")
        self.assertIn("not ours", err)
        self.assertEqual(out, "")

    def test_symlink_is_left_alone_even_if_target_has_marker(self):
        target = self.tmp / "elsewhere.md"
        target.write_text(self.text)
        self.f.parent.mkdir(parents=True)
        self.f.symlink_to(target)
        call(AC.write_generated, self.f, self.text + "changed\n")
        self.assertTrue(self.f.is_symlink())
        self.assertEqual(target.read_text(), self.text)

    def test_dry_run_writes_nothing(self):
        AC.DRY = True
        _, out, _ = call(AC.write_generated, self.f, self.text)
        self.assertFalse(self.f.exists())
        self.assertFalse(self.cdir.exists())
        self.assertIn("would write", out)


class RemoveGeneratedTest(Base):
    def setUp(self):
        super().setUp()
        self.f = self.cdir / "agents" / "a.md"
        self.f.parent.mkdir(parents=True)

    def test_removes_marked_file(self):
        self.f.write_text("x %s y" % AC.MARK)
        _, out, _ = call(AC.remove_generated, self.f, "gone")
        self.assertFalse(self.f.exists())
        self.assertIn("removed ~/.cursor/agents/a.md (gone)", out)

    def test_keeps_user_file(self):
        self.f.write_text("mine")
        call(AC.remove_generated, self.f)
        self.assertEqual(self.f.read_text(), "mine")

    def test_missing_file_is_fine(self):
        _, out, err = call(AC.remove_generated, self.f)
        self.assertEqual((out, err), ("", ""))

    def test_symlink_is_kept(self):
        target = self.tmp / "t.md"
        target.write_text(AC.MARK)
        self.f.symlink_to(target)
        call(AC.remove_generated, self.f)
        self.assertTrue(self.f.is_symlink())
        self.assertTrue(target.exists())

    def test_directory_is_ignored(self):
        d = self.cdir / "agents" / "dir.md"
        d.mkdir()
        call(AC.remove_generated, d)
        self.assertTrue(d.is_dir())

    def test_dry_run_keeps_file(self):
        self.f.write_text(AC.MARK)
        AC.DRY = True
        _, out, _ = call(AC.remove_generated, self.f)
        self.assertTrue(self.f.exists())
        self.assertIn("would remove", out)


# --- cursor_rules, agents -----------------------------------------------------


class CursorRulesTest(Base):
    def style(self, name="Terse", body="Be brief.\n"):
        self.settings(outputStyle=name)
        d = self.claude / "output-styles"
        d.mkdir(exist_ok=True)
        (d / "terse.md").write_text("---\nname: Terse\n---\n\n" + body)

    def test_global_rule_for_each_profile(self):
        self.overlay()
        call(AC.cursor_rules, self.home, "shared")
        f = self.cdir / "rules" / "claude-global.mdc"
        text = f.read_text()
        self.assertTrue(text.startswith("---\ndescription: "))
        self.assertIn("alwaysApply: true", text)
        self.assertIn(AC.MARK, text)
        self.assertNotIn("Off-limits", text)
        self.assertNotIn("owner.md", text)
        call(AC.cursor_rules, self.home, "owner")
        self.assertIn("## Off-limits", f.read_text())
        self.assertIn("claude/CLAUDE.md and ~/.config/dotfiles/owner.md", f.read_text())

    def test_rerun_is_idempotent(self):
        call(AC.cursor_rules, self.home, "shared")
        _, out, _ = call(AC.cursor_rules, self.home, "shared")
        self.assertIn("already set up", out)

    def test_output_style_rule_written_then_removed(self):
        self.style()
        call(AC.cursor_rules, self.home, "shared")
        sf = self.cdir / "rules" / "claude-output-style.mdc"
        self.assertIn("# Reply style: Terse", sf.read_text())
        self.assertIn("claude/output-styles/terse.md", sf.read_text())
        self.settings(outputStyle="default")
        call(AC.cursor_rules, self.home, "shared")
        self.assertFalse(sf.exists())

    def test_user_owned_style_rule_is_not_removed(self):
        sf = self.cdir / "rules" / "claude-output-style.mdc"
        sf.parent.mkdir(parents=True)
        sf.write_text("mine\n")
        call(AC.cursor_rules, self.home, "shared")  # no active style -> remove_generated
        self.assertEqual(sf.read_text(), "mine\n")

    def test_user_owned_global_rule_is_not_overwritten(self):
        gf = self.cdir / "rules" / "claude-global.mdc"
        gf.parent.mkdir(parents=True)
        gf.write_text("mine\n")
        _, _, err = call(AC.cursor_rules, self.home, "shared")
        self.assertEqual(gf.read_text(), "mine\n")
        self.assertIn("not ours", err)

    def test_unknown_style_name_means_no_style_file(self):
        self.settings(outputStyle="Missing")
        call(AC.cursor_rules, self.home, "shared")
        self.assertFalse((self.cdir / "rules" / "claude-output-style.mdc").exists())

    def test_style_matched_by_file_stem(self):
        self.settings(outputStyle="terse")
        d = self.claude / "output-styles"
        d.mkdir()
        (d / "terse.md").write_text("no frontmatter body\n")
        call(AC.cursor_rules, self.home, "shared")
        self.assertIn("no frontmatter body", (self.cdir / "rules" / "claude-output-style.mdc").read_text())

    # user-rules mark logic
    def mark(self, content):
        m = self.home / AC.USER_RULES_MARK
        m.parent.mkdir(parents=True, exist_ok=True)
        m.write_text(content)

    def test_mark_present_removes_our_files_and_skips_when_hash_matches(self):
        self.style()
        call(AC.cursor_rules, self.home, "shared")  # file copies exist
        rules = self.cdir / "rules"
        self.assertTrue((rules / "claude-global.mdc").exists())
        self.mark(AC.sha(AC.user_rules_text(self.home, "shared")) + "\n")
        _, out, err = call(AC.cursor_rules, self.home, "shared")
        self.assertFalse((rules / "claude-global.mdc").exists())
        self.assertFalse((rules / "claude-output-style.mdc").exists())
        self.assertIn("Cursor User Rules", out)
        self.assertIn("already set up", out)
        self.assertEqual(err, "")

    def test_mark_present_warns_when_rules_changed(self):
        self.mark(AC.sha("old text") + "\n")
        _, out, err = call(AC.cursor_rules, self.home, "shared")
        self.assertIn("Run cursor-rules", err)
        self.assertNotIn("already set up", out)
        self.assertFalse((self.cdir / "rules" / "claude-global.mdc").exists())

    def test_mark_present_leaves_user_files_alone(self):
        gf = self.cdir / "rules" / "claude-global.mdc"
        gf.parent.mkdir(parents=True)
        gf.write_text("mine\n")
        self.mark(AC.sha(AC.user_rules_text(self.home, "shared")))
        call(AC.cursor_rules, self.home, "shared")
        self.assertEqual(gf.read_text(), "mine\n")

    def test_mark_hash_depends_on_profile(self):
        self.overlay()
        self.mark(AC.sha(AC.user_rules_text(self.home, "owner")))
        _, _, err = call(AC.cursor_rules, self.home, "shared")
        self.assertIn("Run cursor-rules", err)

    def test_removing_the_mark_brings_the_file_copy_back(self):
        self.mark(AC.sha(AC.user_rules_text(self.home, "shared")))
        call(AC.cursor_rules, self.home, "shared")
        (self.home / AC.USER_RULES_MARK).unlink()
        call(AC.cursor_rules, self.home, "shared")
        self.assertTrue((self.cdir / "rules" / "claude-global.mdc").exists())


class YamlStrTest(unittest.TestCase):
    def test_plain_when_safe(self):
        self.assertEqual(AC.yaml_str("Reviews a diff, then reports."), "Reviews a diff, then reports.")

    def test_quotes_when_needed(self):
        for v in ("a: b", "x #y", "- dash", "'q", "multi\nline", " pad"):
            self.assertEqual(json.loads(AC.yaml_str(v)), v, v)


class CursorAgentsTest(Base):
    def agent(self, fname, text):
        d = self.claude / "agents"
        d.mkdir(exist_ok=True)
        (d / fname).write_text(text)

    def test_description_with_colon_is_quoted_and_round_trips(self):
        self.agent("r.md", "---\nname: r\ndescription: >\n  Use it: after edits\ntools:\n  - Read\n---\nB\n")
        call(AC.cursor_agents, self.home)
        meta, _ = AC.split_frontmatter((self.cdir / "agents" / "r.md").read_text())
        self.assertEqual(meta["description"], "Use it: after edits")
        self.assertEqual(meta["readonly"], "true")

    def test_converts_frontmatter(self):
        self.agent("rev.md", "---\nname: reviewer\ndescription: reviews\nmodel: sonnet\ntools: Read, Grep\n---\n\nBody\n")
        call(AC.cursor_agents, self.home)
        out = (self.cdir / "agents" / "reviewer.md").read_text()
        self.assertTrue(out.startswith("---\nname: reviewer\ndescription: reviews\nmodel: inherit\nreadonly: true\n---\n"))
        self.assertIn(AC.MARK, out)
        self.assertNotIn("sonnet", out)
        self.assertTrue(out.endswith("Body\n"))

    def test_write_capable_tools_are_not_readonly(self):
        self.agent("w.md", "---\nname: w\ntools: Read, Bash\n---\nb\n")
        self.agent("n.md", "---\nname: n\n---\nb\n")  # no tools field
        call(AC.cursor_agents, self.home)
        self.assertNotIn("readonly", (self.cdir / "agents" / "w.md").read_text())
        self.assertNotIn("readonly", (self.cdir / "agents" / "n.md").read_text())

    def test_name_falls_back_to_file_stem_and_readme_is_skipped(self):
        self.agent("stem-name.md", "no frontmatter\n")
        self.agent("README.md", "# readme\n")
        call(AC.cursor_agents, self.home)
        self.assertEqual(sorted(p.name for p in (self.cdir / "agents").iterdir()), ["stem-name.md"])

    def test_stale_generated_agent_removed_user_agent_kept(self):
        self.agent("a.md", "---\nname: a\n---\nb\n")
        call(AC.cursor_agents, self.home)
        mine = self.cdir / "agents" / "mine.md"
        mine.write_text("my agent\n")
        (self.claude / "agents" / "a.md").unlink()
        call(AC.cursor_agents, self.home)
        self.assertFalse((self.cdir / "agents" / "a.md").exists())
        self.assertEqual(mine.read_text(), "my agent\n")

    def test_user_agent_with_same_name_is_not_overwritten(self):
        self.agent("a.md", "---\nname: a\n---\nb\n")
        (self.cdir / "agents").mkdir(parents=True)
        (self.cdir / "agents" / "a.md").write_text("mine\n")
        _, _, err = call(AC.cursor_agents, self.home)
        self.assertEqual((self.cdir / "agents" / "a.md").read_text(), "mine\n")
        self.assertIn("not ours", err)

    def test_no_agents_dir_is_fine(self):
        call(AC.cursor_agents, self.home)
        self.assertFalse((self.cdir / "agents").exists())


# --- JSON merges --------------------------------------------------------------


class SaveJsonTest(Base):
    def test_backup_made_before_change(self):
        p = self.cdir / "x.json"
        wj(p, {"a": 1})
        _, out, _ = call(AC.save_json, p, {"a": 1}, {"a": 2})
        baks = list(self.cdir.glob("x.json.bak-*"))
        self.assertEqual(len(baks), 1)
        self.assertEqual(json.loads(baks[0].read_text()), {"a": 1})
        self.assertEqual(rj(p), {"a": 2})
        self.assertIn("updated", out)

    def test_no_backup_for_new_file(self):
        p = self.cdir / "x.json"
        call(AC.save_json, p, {}, {"a": 1})
        self.assertEqual(list(self.cdir.glob("*.bak-*")), [])

    def test_unchanged_is_skipped(self):
        p = self.cdir / "x.json"
        wj(p, {"a": 1})
        _, out, _ = call(AC.save_json, p, {"a": 1}, {"a": 1})
        self.assertIn("already set up", out)
        self.assertEqual(list(self.cdir.glob("*.bak-*")), [])

    def test_symlink_untouched(self):
        target = self.tmp / "t.json"
        wj(target, {"a": 1})
        p = self.cdir / "x.json"
        p.parent.mkdir(parents=True)
        p.symlink_to(target)
        _, _, err = call(AC.save_json, p, {"a": 1}, {"a": 2})
        self.assertEqual(rj(target), {"a": 1})
        self.assertIn("symlink", err)

    def test_private_mode(self):
        p = self.cdir / "x.json"
        call(AC.save_json, p, {}, {"a": 1}, True)
        self.assertEqual(stat.S_IMODE(p.stat().st_mode), 0o600)

    def test_dry_run(self):
        AC.DRY = True
        p = self.cdir / "x.json"
        _, out, _ = call(AC.save_json, p, {}, {"a": 1})
        self.assertFalse(p.exists())
        self.assertIn("would update", out)


class CursorHooksTest(Base):
    path = property(lambda self: self.cdir / "hooks.json")

    def run_hooks(self):
        return call(AC.cursor_hooks, self.home)

    def test_creates_file_with_all_events(self):
        self.run_hooks()
        data = rj(self.path)
        self.assertEqual(data["version"], 1)
        self.assertEqual(set(data["hooks"]), set(AC.HOOK_EVENTS))
        for event, (mode, matcher) in AC.HOOK_EVENTS.items():
            (entry,) = data["hooks"][event]
            self.assertTrue(entry["command"].startswith("test -r "))
            self.assertIn("|| exit 0;", entry["command"])  # a missing script must not deny
            self.assertTrue(entry["command"].endswith("python3 %s %s" % (self.guard, mode)))
            self.assertEqual(entry["timeout"], 10)
            if matcher:
                self.assertEqual(entry["matcher"], matcher)
            else:
                self.assertNotIn("matcher", entry)

    def test_user_entries_and_keys_are_kept(self):
        mine = {"command": "echo mine", "timeout": 3}
        wj(self.path, {"version": 2, "hooks": {"preToolUse": [mine], "stop": [mine]}, "extra": True})
        self.run_hooks()
        data = rj(self.path)
        self.assertEqual(data["version"], 2)  # user's version wins
        self.assertTrue(data["extra"])
        self.assertEqual(data["hooks"]["stop"], [mine])
        self.assertEqual(data["hooks"]["preToolUse"][0], mine)
        self.assertEqual(len(data["hooks"]["preToolUse"]), 2)

    def test_our_entry_keeps_its_place_between_user_entries(self):
        self.run_hooks()
        data = rj(self.path)
        a, b = {"command": "echo a"}, {"command": "echo b"}
        data["hooks"]["preCompact"] = [a] + data["hooks"]["preCompact"] + [b]
        wj(self.path, data)
        self.run_hooks()
        cmds = [e["command"] for e in rj(self.path)["hooks"]["preCompact"]]
        self.assertEqual(cmds[0], "echo a")
        self.assertIn(AC.GUARD_TAG, cmds[1])
        self.assertEqual(cmds[2], "echo b")

    def test_rerun_is_idempotent_and_makes_no_backup(self):
        self.run_hooks()
        first = self.path.read_text()
        _, out, _ = self.run_hooks()
        self.assertEqual(self.path.read_text(), first)
        self.assertIn("already set up", out)
        self.assertEqual(list(self.cdir.glob("hooks.json.bak-*")), [])

    def test_old_entries_for_a_moved_script_are_replaced(self):
        old = {"command": "python3 /old/home/cursor/hooks/guard_cursor.py prompt"}
        wj(self.path, {"version": 1, "hooks": {"beforeSubmitPrompt": [old]}})
        self.run_hooks()
        cmds = [e["command"] for e in rj(self.path)["hooks"]["beforeSubmitPrompt"]]
        self.assertEqual(len(cmds), 1)
        self.assertNotIn("/old/home", cmds[0])
        self.assertEqual(len(list(self.cdir.glob("hooks.json.bak-*"))), 1)

    def test_missing_guard_removes_our_entries_and_empty_events(self):
        self.run_hooks()
        mine = {"command": "echo mine"}
        data = rj(self.path)
        data["hooks"]["preCompact"].append(mine)
        wj(self.path, data)
        self.guard.unlink()
        self.run_hooks()
        hooks = rj(self.path)["hooks"]
        self.assertEqual(hooks, {"preCompact": [mine]})

    def test_missing_guard_and_no_file_still_writes_skeleton(self):
        self.guard.unlink()
        self.run_hooks()
        self.assertEqual(rj(self.path), {"version": 1, "hooks": {}})

    def test_invalid_json_left_untouched(self):
        self.path.parent.mkdir(parents=True)
        self.path.write_text("{oops")
        _, _, err = self.run_hooks()
        self.assertEqual(self.path.read_text(), "{oops")
        self.assertIn("not valid JSON", err)

    def test_non_object_hooks_left_untouched(self):
        wj(self.path, {"hooks": []})
        _, _, err = self.run_hooks()
        self.assertEqual(rj(self.path), {"hooks": []})
        self.assertIn("non-object", err)

    def test_path_with_spaces_is_quoted(self):
        spaced = self.repo / "my cursor" / "cursor" / "hooks" / "guard_cursor.py"
        spaced.parent.mkdir(parents=True)
        spaced.write_text("#")
        with mock.patch.object(AC, "GUARD", spaced):
            self.run_hooks()
            cmd = rj(self.path)["hooks"]["preCompact"][0]["command"]
            self.assertIn("'%s'" % spaced, cmd)
            # a second run still recognises its own quoted entries
            self.run_hooks()
            self.assertEqual(len(rj(self.path)["hooks"]["preCompact"]), 1)

    def test_dry_run_writes_nothing(self):
        AC.DRY = True
        self.run_hooks()
        self.assertFalse(self.cdir.exists())


class CursorMcpTest(Base):
    path = property(lambda self: self.cdir / "mcp.json")

    def ours(self, servers):
        wj(self.cursor_src / "mcp.json", {"mcpServers": servers})

    def test_adds_ours_to_new_file(self):
        self.ours({"docs": {"url": "https://example.invalid/mcp"}})
        call(AC.cursor_mcp, self.home)
        self.assertEqual(rj(self.path), {"mcpServers": {"docs": {"url": "https://example.invalid/mcp"}}})

    def test_user_entry_of_same_name_wins_and_others_kept(self):
        self.ours({"docs": {"url": "ours"}, "extra": {"command": "x"}})
        wj(self.path, {"mcpServers": {"docs": {"url": "users"}, "mine": {"command": "m"}}, "other": 1})
        call(AC.cursor_mcp, self.home)
        data = rj(self.path)
        self.assertEqual(data["mcpServers"]["docs"], {"url": "users"})
        self.assertEqual(data["mcpServers"]["mine"], {"command": "m"})
        self.assertEqual(data["mcpServers"]["extra"], {"command": "x"})
        self.assertEqual(data["other"], 1)

    def test_idempotent(self):
        self.ours({"docs": {"url": "ours"}})
        call(AC.cursor_mcp, self.home)
        _, out, _ = call(AC.cursor_mcp, self.home)
        self.assertIn("already set up", out)
        self.assertEqual(list(self.cdir.glob("mcp.json.bak-*")), [])

    def test_no_source_file_keeps_user_file_content(self):
        wj(self.path, {"mcpServers": {"mine": {}}})
        call(AC.cursor_mcp, self.home)
        self.assertEqual(rj(self.path), {"mcpServers": {"mine": {}}})

    def test_invalid_json_left_untouched(self):
        self.ours({"docs": {}})
        self.path.parent.mkdir(parents=True)
        self.path.write_text("[]")
        _, _, err = call(AC.cursor_mcp, self.home)
        self.assertEqual(self.path.read_text(), "[]")
        self.assertIn("not valid JSON", err)

    def test_shared_profile_skips_without_opt_in(self):
        self.ours({"docs": {}})
        _, out, _ = call(AC.cursor_mcp, self.home, "shared")
        self.assertFalse(self.path.exists())
        self.assertIn("opt-in", out)

    def test_shared_profile_opt_in_file(self):
        self.ours({"docs": {}})
        (self.home / ".config" / "dotfiles").mkdir(parents=True)
        (self.home / ".config" / "dotfiles" / "mcp").write_text("on\n")
        call(AC.cursor_mcp, self.home, "shared")
        self.assertIn("docs", rj(self.path)["mcpServers"])

    def test_env_wins_over_file_and_profile(self):
        self.ours({"docs": {}})
        (self.home / ".config" / "dotfiles").mkdir(parents=True)
        (self.home / ".config" / "dotfiles" / "mcp").write_text("on\n")
        os.environ["DOTFILES_MCP"] = "off"
        call(AC.cursor_mcp, self.home, "owner")
        self.assertFalse(self.path.exists())
        os.environ["DOTFILES_MCP"] = "1"
        call(AC.cursor_mcp, self.home, "shared")
        self.assertTrue(self.path.exists())

    def test_change_makes_backup(self):
        self.ours({"docs": {}})
        wj(self.path, {"mcpServers": {}})
        call(AC.cursor_mcp, self.home)
        self.assertEqual(len(list(self.cdir.glob("mcp.json.bak-*"))), 1)


class CliConfigTest(Base):
    path = property(lambda self: self.cdir / "cli-config.json")
    side = property(lambda self: self.cdir / AC.MANAGED)

    def run_cfg(self):
        return call(AC.cursor_cli_config, self.home)

    def test_defaults_on_empty_home(self):
        self.run_cfg()
        data = rj(self.path)
        self.assertEqual(data["version"], 1)
        self.assertEqual(data["editor"], {"vimMode": False})
        self.assertEqual(data["permissions"], {"allow": [], "deny": []})
        self.assertEqual(data["attribution"], {"attributeCommitsToAgent": False, "attributePRsToAgent": False})
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o600)

    def test_user_values_win_but_attribution_is_forced_off(self):
        wj(self.path, {
            "version": 7, "editor": {"vimMode": True}, "model": {"id": "x"},
            "attribution": {"attributeCommitsToAgent": True, "attributePRsToAgent": True, "keep": "me"},
        })
        self.run_cfg()
        data = rj(self.path)
        self.assertEqual(data["version"], 7)
        self.assertEqual(data["editor"], {"vimMode": True})
        self.assertEqual(data["model"], {"id": "x"})
        self.assertIs(data["attribution"]["attributeCommitsToAgent"], False)
        self.assertIs(data["attribution"]["attributePRsToAgent"], False)
        self.assertEqual(data["attribution"]["keep"], "me")

    def test_translated_rules_added_and_user_rules_kept(self):
        self.perms(allow=["Bash(git status:*)", "Read(./src/**)", "Glob(x)"], deny=["Read(./.env)"])
        wj(self.path, {"permissions": {"allow": ["Shell(mine)"], "deny": ["Read(secret)"], "other": 1}})
        self.run_cfg()
        p = rj(self.path)["permissions"]
        self.assertEqual(p["allow"], ["Shell(mine)", "Shell(git status)", "Read(src/**)"])
        self.assertEqual(p["deny"], ["Read(secret)", "Read(**/.env)"])
        self.assertEqual(p["other"], 1)

    def test_sidecar_records_what_we_added(self):
        self.perms(allow=["Bash(git status:*)", "Nope(x)"], deny=["Read(./.env)"])
        self.run_cfg()
        self.assertEqual(rj(self.side), {"allow": ["Shell(git status)"], "deny": ["Read(**/.env)"]})

    def test_rerun_is_idempotent(self):
        self.perms(allow=["Bash(git status:*)"], deny=["Read(./.env)"])
        wj(self.path, {"permissions": {"allow": ["Shell(mine)"]}})
        self.run_cfg()
        cfg, sidecar = self.path.read_text(), self.side.read_text()
        baks = len(list(self.cdir.glob("cli-config.json.bak-*")))
        self.assertEqual(baks, 1)
        _, out, _ = self.run_cfg()
        self.assertEqual(self.path.read_text(), cfg)
        self.assertEqual(self.side.read_text(), sidecar)
        self.assertEqual(len(list(self.cdir.glob("cli-config.json.bak-*"))), baks)
        self.assertIn("already set up", out)

    def test_no_duplicates_when_user_already_has_the_rule(self):
        self.perms(allow=["Bash(git status:*)"])
        wj(self.path, {"permissions": {"allow": ["Shell(git status)"]}})
        self.run_cfg()
        self.assertEqual(rj(self.path)["permissions"]["allow"], ["Shell(git status)"])

    def test_stale_rule_we_owned_is_removed_user_rules_survive(self):
        self.perms(allow=["Bash(git status:*)", "Bash(npm test:*)"])
        self.run_cfg()
        data = rj(self.path)
        data["permissions"]["allow"].append("Shell(mine)")
        wj(self.path, data)
        self.perms(allow=["Bash(git status:*)"])  # npm test dropped from the source
        self.run_cfg()
        self.assertEqual(rj(self.path)["permissions"]["allow"], ["Shell(git status)", "Shell(mine)"])
        self.assertEqual(rj(self.side)["allow"], ["Shell(git status)"])

    def test_rule_never_owned_is_never_removed(self):
        self.perms(allow=["Bash(git status:*)"])
        wj(self.path, {"permissions": {"allow": ["Shell(npm test)"]}})
        self.run_cfg()
        self.perms(allow=[])
        self.run_cfg()
        allow = rj(self.path)["permissions"]["allow"]
        self.assertEqual(allow, ["Shell(npm test)"])  # ours (git status) went, theirs stayed

    def test_deny_rules_are_tracked_separately_from_allow(self):
        self.perms(allow=["Read(./a)"], deny=["Read(./b)"])
        self.run_cfg()
        self.perms(allow=[], deny=[])
        self.run_cfg()
        self.assertEqual(rj(self.path)["permissions"], {"allow": [], "deny": []})
        self.assertEqual(rj(self.side), {"allow": [], "deny": []})

    def test_user_deleting_our_rule_gets_it_back(self):
        self.perms(allow=["Bash(git status:*)"])
        self.run_cfg()
        data = rj(self.path)
        data["permissions"]["allow"] = []
        wj(self.path, data)
        self.run_cfg()
        self.assertEqual(rj(self.path)["permissions"]["allow"], ["Shell(git status)"])

    def test_user_rule_equal_to_ours_is_kept_once(self):
        # A hand-written rule that equals one of ours stays, and is not added twice.
        self.perms(allow=["Bash(git status:*)"])
        wj(self.path, {"permissions": {"allow": ["Shell(git status)"]}})
        self.run_cfg()
        self.assertEqual(rj(self.path)["permissions"]["allow"], ["Shell(git status)"])

    def test_corrupt_sidecar_is_survivable(self):
        self.perms(allow=["Bash(git status:*)"])
        self.side.parent.mkdir(parents=True)
        self.side.write_text("{not json")
        wj(self.path, {"permissions": {"allow": ["Shell(mine)"]}})
        _, _, err = self.run_cfg()
        self.assertEqual(err, "")
        self.assertEqual(rj(self.path)["permissions"]["allow"], ["Shell(mine)", "Shell(git status)"])
        self.assertEqual(rj(self.side)["allow"], ["Shell(git status)"])

    def test_sidecar_not_written_when_config_is_a_symlink(self):
        self.perms(allow=["Bash(git status:*)"])
        target = self.tmp / "real.json"
        wj(target, {"permissions": {"allow": []}})
        self.path.parent.mkdir(parents=True)
        self.path.symlink_to(target)
        _, _, err = self.run_cfg()
        self.assertIn("symlink", err)
        self.assertFalse(self.side.exists())
        self.assertEqual(rj(target), {"permissions": {"allow": []}})

    def test_invalid_config_json_left_untouched_and_no_sidecar(self):
        self.perms(allow=["Bash(git status:*)"])
        self.path.parent.mkdir(parents=True)
        self.path.write_text("nope")
        _, _, err = self.run_cfg()
        self.assertEqual(self.path.read_text(), "nope")
        self.assertIn("not valid JSON", err)
        self.assertFalse(self.side.exists())

    def test_dry_run_writes_neither_config_nor_sidecar(self):
        self.perms(allow=["Bash(git status:*)"])
        AC.DRY = True
        self.run_cfg()
        self.assertFalse(self.cdir.exists())

    def test_statusline_added_only_when_script_exists(self):
        self.run_cfg()
        self.assertNotIn("statusLine", rj(self.path))
        sl = self.home / ".claude" / "statusline.sh"
        sl.parent.mkdir(parents=True)
        sl.write_text("#!/bin/sh\n")
        self.run_cfg()
        self.assertEqual(rj(self.path)["statusLine"], {"type": "command", "command": str(sl), "padding": 0})

    def test_user_statusline_wins(self):
        sl = self.home / ".claude" / "statusline.sh"
        sl.parent.mkdir(parents=True)
        sl.write_text("#!/bin/sh\n")
        wj(self.path, {"statusLine": {"type": "command", "command": "mine"}})
        self.run_cfg()
        self.assertEqual(rj(self.path)["statusLine"]["command"], "mine")

    def test_missing_source_settings_is_fine(self):
        self.run_cfg()
        self.assertEqual(rj(self.path)["permissions"], {"allow": [], "deny": []})


# --- commands -----------------------------------------------------------------


class CmdCursorTest(Base):
    def test_full_run_creates_everything_and_never_writes_claude_dir(self):
        self.perms(allow=["Bash(ls:*)"])
        wj(self.cursor_src / "mcp.json", {"mcpServers": {"docs": {"url": "u"}}})
        (self.claude / "agents").mkdir()
        (self.claude / "agents" / "a.md").write_text("---\nname: a\n---\nb\n")
        rc, out, err = call(AC.main, ["cursor", "--profile", "shared"])
        self.assertEqual(rc, 0)
        self.assertEqual(err, "")
        for rel in ("rules/claude-global.mdc", "agents/a.md", "hooks.json", "cli-config.json", AC.MANAGED):
            self.assertTrue((self.cdir / rel).exists(), rel)
        self.assertFalse((self.cdir / "mcp.json").exists())  # MCP is opt-in on shared
        self.assertFalse((self.home / ".claude").exists())
        self.assertNotIn("Off-limits", (self.cdir / "rules" / "claude-global.mdc").read_text())

    def test_owner_profile_gets_overlay_and_mcp(self):
        self.overlay()
        wj(self.cursor_src / "mcp.json", {"mcpServers": {"docs": {"url": "u"}}})
        call(AC.main, ["cursor", "--profile", "owner"])
        self.assertTrue((self.cdir / "mcp.json").exists())
        self.assertIn("## Off-limits", (self.cdir / "rules" / "claude-global.mdc").read_text())

    def test_owner_rule_file_is_private_and_names_both_sources(self):
        self.overlay()
        call(AC.main, ["cursor", "--profile", "owner"])
        rule = self.cdir / "rules" / "claude-global.mdc"
        self.assertEqual(stat.S_IMODE(rule.stat().st_mode), 0o600)
        self.assertIn(f"and ~/{AC.OWNER_MD}. Edit those files", rule.read_text())

    def test_owner_rule_file_turns_private_when_the_text_is_unchanged(self):
        self.overlay()
        call(AC.main, ["cursor", "--profile", "owner"])
        rule = self.cdir / "rules" / "claude-global.mdc"
        rule.chmod(0o644)
        call(AC.main, ["cursor", "--profile", "owner"])
        self.assertEqual(stat.S_IMODE(rule.stat().st_mode), 0o600)

    def test_owner_without_overlay_warns_and_names_one_source(self):
        wj(self.cursor_src / "mcp.json", {"mcpServers": {"docs": {"url": "u"}}})
        rc, _, err = call(AC.main, ["cursor", "--profile", "owner"])
        self.assertEqual(rc, 0)
        self.assertIn("no owner-only rules", err)
        rule = self.cdir / "rules" / "claude-global.mdc"
        self.assertIn("from claude/CLAUDE.md. Edit that file", rule.read_text())
        self.assertEqual(stat.S_IMODE(rule.stat().st_mode), 0o644)

    def test_second_run_is_all_skips(self):
        self.perms(allow=["Bash(ls:*)"])
        call(AC.main, ["cursor"])
        rc, out, err = call(AC.main, ["cursor"])
        self.assertEqual(rc, 0)
        self.assertNotIn("wrote", out)
        self.assertNotIn("updated", out)
        self.assertEqual(err, "")

    def test_dry_run_leaves_home_empty(self):
        rc, out, _ = call(AC.main, ["cursor", "--dry-run"])
        self.assertEqual(rc, 0)
        self.assertEqual(list(self.home.iterdir()), [])
        self.assertIn("would", out)

    def test_one_failing_part_does_not_stop_the_others(self):
        wj(self.cdir / "cli-config.json", {"permissions": {"allow": "not a list"}})
        rc, _, err = call(AC.main, ["cursor"])
        self.assertEqual(rc, 1)
        self.assertIn("skipped a part", err)
        self.assertTrue((self.cdir / "hooks.json").exists())
        self.assertTrue((self.cdir / "rules" / "claude-global.mdc").exists())


class CmdUserRulesTest(Base):
    def test_print_mode_writes_text_to_stdout_only(self):
        rc, out, _ = call(AC.main, ["user-rules", "--profile", "shared"])
        self.assertEqual(rc, 0)
        self.assertEqual(out, AC.user_rules_text(self.home, "shared"))
        self.assertFalse((self.home / AC.USER_RULES_MARK).exists())

    def test_default_profile_comes_from_env(self):
        self.overlay()
        os.environ["DOTFILES_PROFILE"] = "owner"
        _, out, _ = call(AC.main, ["user-rules"])
        self.assertIn("## Off-limits", out)

    def test_copy_sends_text_to_clipboard_writes_mark_and_drops_file_copy(self):
        call(AC.cursor_rules, self.home, "shared")
        gf = self.cdir / "rules" / "claude-global.mdc"
        self.assertTrue(gf.exists())
        mine = self.cdir / "rules" / "claude-output-style.mdc"
        mine.write_text("user's own\n")
        with mock.patch("subprocess.run") as run:  # pbcopy is the only thing faked
            rc, out, _ = call(AC.main, ["user-rules", "--profile", "shared", "--copy"])
        self.assertEqual(rc, 0)
        args, kwargs = run.call_args
        self.assertEqual(args[0], ["pbcopy"])
        text = AC.user_rules_text(self.home, "shared")
        self.assertEqual(kwargs["input"], text.encode())
        self.assertEqual((self.home / AC.USER_RULES_MARK).read_text(), hashlib.sha256(text.encode()).hexdigest() + "\n")
        self.assertFalse(gf.exists())
        self.assertEqual(mine.read_text(), "user's own\n")  # not ours, kept
        self.assertIn("copied the rules", out)

    def test_after_copy_cursor_rules_skips_then_warns_on_change(self):
        with mock.patch("subprocess.run"):
            call(AC.main, ["user-rules", "--profile", "shared", "--copy"])
        _, out, err = call(AC.cursor_rules, self.home, "shared")
        self.assertIn("already set up", out)
        self.assertEqual(err, "")
        (self.claude / "CLAUDE.md").write_text(CLAUDE_MD + "\n## New\n\nmore\n")
        _, out, err = call(AC.cursor_rules, self.home, "shared")
        self.assertIn("Run cursor-rules", err)
        self.assertFalse((self.cdir / "rules" / "claude-global.mdc").exists())

    def test_full_cursor_run_after_copy_creates_no_rule_files(self):
        with mock.patch("subprocess.run"):
            call(AC.main, ["user-rules", "--profile", "shared", "--copy"])
        call(AC.main, ["cursor"])
        self.assertFalse((self.cdir / "rules" / "claude-global.mdc").exists())
        self.assertTrue((self.cdir / "hooks.json").exists())


class MainTest(Base):
    def test_requires_a_subcommand(self):
        with self.assertRaises(SystemExit):
            call(AC.main, [])

    def test_rejects_unknown_profile(self):
        with self.assertRaises(SystemExit):
            call(AC.main, ["cursor", "--profile", "root"])

    def test_dry_flag_is_set_per_invocation(self):
        call(AC.main, ["cursor", "--dry-run"])
        self.assertTrue(AC.DRY)
        call(AC.main, ["cursor"])
        self.assertFalse(AC.DRY)


if __name__ == "__main__":
    unittest.main()
