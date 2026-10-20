# Set this up on a friend's machine

A runbook for Claude. The friend opens a Claude Code session on their own machine and asks Claude
to follow this file. Claude reads it from the repo page before the clone, and from
`~/dotfiles/docs/FRIENDS-SETUP.md` after step 2. Claude follows the sections in order.

## How to run this file

Each step starts with a tag.

- `[RUN]` Run the command yourself. Show the last line of its output.
- `[ASK]` Ask the friend. Give two to four options, the recommended one first, and ask all the
  questions of one section in one go. Use the default when the friend says "you pick".
- `[CONFIRM]` Show what you are about to do, then wait for a yes.

Rules for the whole run:

- Stop on any failure. Quote the failing line, say what it means, and ask. Do not skip the step.
- Never invent a value. A name, an email, a time zone or a plan comes from the friend or from the
  machine (`git config`, `date`), never from a guess.
- Never ask for a password or a token in the chat, and never print one. Logins happen in the
  friend's own terminal or browser.
- Never open `~/.config/zsh/local.zsh`. It holds secrets.
- Keep to the OS of the machine. Run `uname -s` first: `Darwin` is macOS, `Linux` is Linux,
  `MINGW...` or `MSYS...` is Windows (Git Bash).
- Fill in the table at the end as you go.

## 1. Prerequisites

`[RUN]` Check what is there. Report each line as found or missing.

| OS | Needs | Check |
|----|-------|-------|
| macOS | the Terminal app, a Mac password, internet | `uname -s` prints `Darwin` |
| Linux | Debian, Ubuntu, Fedora, Arch or Alpine; a normal user with `sudo` (Alpine: root or `sudo`); `curl` and `git` | `command -v curl git sudo` |
| Windows | Git Bash; `git` and `curl` on PATH; Developer Mode on (Settings, System, For developers) | `command -v git curl` |

`[CONFIRM]` If something is missing, show the fix (the README "Set up" section of the OS names it)
and wait until the friend has done it.

## 2. Bootstrap

The one-line setup is in the [README](../README.md): pick the section for the OS ("Set up macOS",
"Set up Linux" or "Set up Windows"). Do not copy the line from here, the README is the one place
that holds it.

`[CONFIRM]` Tell the friend to paste that line in a plain terminal, not in a Claude Code window.
The run asks questions and may ask for a password, and a Claude Code window can race with the
files that the setup writes. Wait until the friend says it has finished.

`[RUN]` Check that it cloned and ran:

```sh
test -x ~/dotfiles/dotfiles && echo found
```

Expected: `found`. If not, stop and ask for the last lines the friend saw.

## 3. First run

The bootstrap already ran `dotfiles` once. Its questions are:

- the git name and email, for commits
- the settings menu (presets, plan, time zone and more; step 4)
- on a Mac, whether to install the Claude desktop app

`[RUN]` Read the last line of that run if the friend pasted it. It should say `dotfiles: done`.
A line with warnings is fine: read the `!` lines above it and report them.

`[RUN]` Run it again to see that nothing changes:

```sh
bash ~/dotfiles/dotfiles
```

Expected: about 20 short lines and a closing `dotfiles: done` line. On a first run the setup can
take 20 to 40 minutes on a Mac, and about 10 on Windows.

## 4. Presets and settings

Presets are opt-in working habits. A preset that is off costs no tokens. The list, one line each:

| Preset | What it does |
|--------|--------------|
| `workflow` | speed over ceremony, hand-offs in files, briefs, the agent set |
| `pace` | make the weekly budget last the work days, with helper caps |
| `models` | effort and model picks |
| `clock` | say the local time and an estimate with a range |
| `plain-writing` | no em dashes, short plain sentences |
| `no-ai-attribution` | no assistant co-author or "generated with" line in commits and PRs |
| `ship-to-main` | work lands on main in one commit. Off by default for a friend: a push to a work repo's main can hurt |

`[ASK]` Ask these together:

1. Which presets? Options: all but `ship-to-main` (recommended), all, none, pick by name.
2. Which Claude plan? Options: `pro`, `max5`, `max20`, `team`, `api`, `unknown` (recommended when the friend
   does not know). It sets the weekly pace budget.
3. Which days do they work? Examples: `mon-fri` (default), `mon-sat`, `sun-thu`.
4. Which time zone? An IANA name such as `UTC` or `Area/City`, or empty for the system zone.
5. Colors? Options: `copper` (default) or `plain`.

`[CONFIRM]` Show the lines you will write to `~/.config/dotfiles/settings`, one `NAME="value"` per
line, for example:

```
PRESETS="workflow pace models clock plain-writing no-ai-attribution"
PLAN="pro"
WORK_DAYS="mon-fri"
TIMEZONE=""
THEME="copper"
```

Leave a name out for the default. An empty value is the default too. The friend can also run
`bash ~/dotfiles/dotfiles settings` in a terminal for the same menu.

`[RUN]` Write the lines (add to the file, keep the existing ones), then run `bash ~/dotfiles/dotfiles`
once more so the links and generated files follow. All names are in `settings.example`.

## 5. Sign-ins

The tools need a login. The friend does these in their own terminal. You never see a password or a token.

`[ASK]` Which to do now? Options: all (recommended), GitHub only, later.

`[CONFIRM]` For each pick, name the command and wait until the friend says it is done:

| Tool | Command | What happens |
|------|---------|--------------|
| GitHub | `gh auth login` | a browser opens |
| Claude Code | `claude auth login` | a browser opens |
| Opencode | `opencode auth login` | a browser or a key prompt |
| Cursor | sign in inside the app | the app asks |

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

It prints one line per check, `PASS <name>` or `FAIL <name>: <why>`, and exits 1 when one fails. It
changes nothing. The eight checks:

| Check | Looks at | Usual fix on FAIL |
|-------|----------|-------------------|
| `links` | every linked file points into the repo | `bash ~/dotfiles/dotfiles` |
| `settings-built` | `~/.claude/settings.json` exists and every hook runs `rust-bin` | `bash ~/dotfiles/dotfiles` |
| `bins-fresh` | each Rust program is built and newer than its source | `bash ~/dotfiles/dotfiles` |
| `guard-selftest` | the guard blocks what it should and lets `ls` through | `bash ~/dotfiles/dotfiles`, then run doctor again |
| `presets` | the preset links equal the `PRESETS` setting | `bash ~/dotfiles/dotfiles` |
| `generated` | the Cursor rule and Opencode files carry the `generated-by: dotfiles` marker | move away a file of the friend's own with that name, then `bash ~/dotfiles/dotfiles` |
| `timezone` | `TIMEZONE` is empty or a real zone name | fix the value in the settings (macOS and Linux) |
| `no-leak` | no private word in a tracked file | remove the word from that file |

On a FAIL: apply the fix in the last column, run doctor again, and stop after a second failure of
the same check. Quote the line to the friend.

## 7. Result

Copy this table into the reply and fill it in. `PASS` and `FAIL` come from the doctor lines. The
last three rows are yes or no.

| Item | Result |
|------|--------|
| OS | |
| `links` | |
| `settings-built` | |
| `bins-fresh` | |
| `guard-selftest` | |
| `presets` | |
| `generated` | |
| `timezone` | |
| `no-leak` | |
| Signed in to GitHub | |
| Signed in to Claude Code | |
| Opened a new terminal and ran `claude`, then `/ps:` shows skills | |

Last, tell the friend: open a new terminal, run `claude`, and type `/ps:` to see the skills. Run
`dots` any time to update and `dots doctor` any time to check.
