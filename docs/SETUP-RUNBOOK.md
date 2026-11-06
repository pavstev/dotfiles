# Set up a new machine with Claude

A runbook for Claude. The user opens a Claude Code session on their own machine and asks Claude
to follow this file. Claude reads it from the repo page before the clone, and from
`~/dotfiles/docs/SETUP-RUNBOOK.md` after step 2. Follow the sections in order.

## How to run this file

Each step starts with a tag.

- `[RUN]` Run the command yourself. Show the last line of its output.
- `[ASK]` Ask the user. Give two to four options, the recommended one first. Ask all the
  questions of one section in one go. Use the default when the user says "you pick".
- `[CONFIRM]` Show what you are about to do, then wait for a yes.

Rules for the whole run:

- Stop on any failure. Quote the failing line, say what it means, and ask. Do not skip the step.
- Never invent a value. A name, an email, a time zone or a plan comes from the user or from the
  machine (`git config`, `date`).
- Never ask for a password or a token in the chat, and never print one. Logins happen in the
  user's own terminal or browser.
- Never open `~/.config/zsh/local.zsh`. It holds secrets.
- Run `uname -s` first: `Darwin` is macOS, `Linux` is Linux, `MINGW...` or `MSYS...` is Windows
  (Git Bash). Keep to that OS.
- Fill in the table at the end as you go.

## 1. Prerequisites

`[RUN]` Check what is there. Report each line as found or missing.

| OS | Needs | Check |
|----|-------|-------|
| macOS | the Terminal app, a Mac password, internet | `uname -s` prints `Darwin` |
| Linux | Debian, Ubuntu, Fedora, Arch or Alpine; a normal user with `sudo` (Alpine: root or `sudo`); `curl` and `git` | `command -v curl git sudo` |
| Windows | Git Bash; `git` and `curl` on PATH; Developer Mode on (Settings, System, For developers) | `command -v git curl` |

`[CONFIRM]` If something is missing, show the fix (the OS section of [Install](install.md) names
it) and wait until the user has done it.

## 2. Bootstrap

The one-line setup is in [Install](install.md). Use the section for the OS ("Set up macOS", "Set
up Linux" or "Set up Windows"). Do not copy the line into this file: read it from Install.

`[CONFIRM]` Tell the user to paste that line in a plain terminal, not in a Claude Code window.
The run asks questions and may ask for a password, and a Claude Code window can race with the
files the setup writes. Wait until the user says it has finished.

`[RUN]` Check that it cloned and ran:

```sh
test -x ~/dotfiles/dotfiles && echo found
```

Expected: `found`. If not, stop and ask for the last lines the user saw.

## 3. First run

The bootstrap already ran `dotfiles` once. Its questions are:

- the git name and email, for commits
- the settings menu (step 4)
- the looks: Starship prompt, VS Code and Cursor settings, Nerd Font. All are optional. They are
  asked once more after an update when not set yet.
- on a Mac, whether to install the Claude desktop app

`[RUN]` If the user pasted the output, read its last line. It should say `dotfiles: done in <time>`.
A line with warnings is fine: read the `!` lines above it and report them. Stuck for several
minutes ([Troubleshooting](troubleshooting.md)): Ctrl-C, then `bash ~/dotfiles/dotfiles -v`.

`[RUN]` Run it again to see that nothing changes:

```sh
bash ~/dotfiles/dotfiles
```

Expected: about 20 short lines and a closing `dotfiles: done` line. A first run takes 20 to 40
minutes on a Mac and 5 to 10 on Windows.

## 4. Presets and settings

Presets are opt-in working habits from the ps plugin. What each one does: [Presets](https://github.com/pavstev/agent-kit/tree/main/docs/presets).
`ship-to-main` is off by default: a push to a work repo's main can hurt.

`[ASK]` Ask these together:

1. Which presets? Options: all but `ship-to-main` (recommended), all, none, pick by name.
2. Which time zone? An IANA name such as `UTC` or `Area/City`, or empty for the system zone.
3. Colors? Options: `copper` (default) or `plain`.

`[CONFIRM]` Show the lines you will write to `~/.config/dotfiles/settings`, one `NAME="value"` per
line, for example:

```
PRESETS="workflow models clock plain-writing no-ai-attribution"
TIMEZONE=""
THEME="copper"
```

Leave a name out, or give it an empty value, for the default. The user can run
`bash ~/dotfiles/dotfiles settings` in a terminal for the same menu.

`[RUN]` Add the lines to the file and keep the existing ones. Then run `bash ~/dotfiles/dotfiles`
once more so the links and generated files follow. All names are in `settings.example`.

## 5. Sign-ins

The tools need a login. The user does these in their own terminal. You never see a password or a
token.

`[ASK]` Which to do now? Options: all (recommended), GitHub only, later.

`[CONFIRM]` For each pick, name the command and wait until the user says it is done:

- GitHub: `gh auth login` (a browser opens)
- Claude Code: `claude auth login` (a browser opens)
- Opencode: `opencode auth login` (a browser or a key prompt)
- Cursor: sign in inside the app

`[RUN]` Check the logins that have a status command. Report only yes or no, never the output of a
token command.

```sh
gh auth status
claude auth status
```

## 6. Check with `dots doctor`

`[RUN]`

```sh
bash ~/dotfiles/dotfiles doctor
```

It prints `PASS <name>` or `FAIL <name>: <why>` per check and changes nothing. What each check
looks at: [Features](features.md#daily-workflow). The usual fix on FAIL:

- `links`, `settings-built`, `bins-fresh`: `bash ~/dotfiles/dotfiles`
- `plugin`: start Claude Code once (the plugin fetches its binaries), then `bash ~/dotfiles/dotfiles`
- `timezone`: fix the value in the settings (macOS and Linux)
- `no-leak`: remove the private word from that file

On a FAIL: apply the fix, run doctor again, and stop after a second failure of the same check.
Quote the line to the user.

## 7. Result

Copy this table into the reply and fill it in. `PASS` and `FAIL` come from the doctor lines. The
last three rows are yes or no.

| Item | Result |
|------|--------|
| OS | |
| `links` | |
| `settings-built` | |
| `bins-fresh` | |
| `plugin` | |
| `timezone` | |
| `no-leak` | |
| Signed in to GitHub | |
| Signed in to Claude Code | |
| Opened a new terminal and ran `claude`, then `/ps:` shows skills | |

Last, tell the user: open a new terminal, run `claude`, and type `/ps:` to see the skills. Run
`dots` any time to update and `dots doctor` any time to check. `dots uninstall` takes it all back
([Customize](customize.md#uninstall)).
