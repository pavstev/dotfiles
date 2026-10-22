<div align="center">

<img src="assets/logo.svg" width="112" height="112" alt="dotfiles logo: a prompt chevron, a dot and a blinking cursor on a night sky">

# dotfiles

**AI-ready dotfiles for macOS, Linux and Windows.**<br>
One command sets up Claude Code, Cursor and Opencode with shared skills, agents, rules and guard hooks, plus a fast terminal.

[![macOS](https://img.shields.io/badge/macOS-07080F?style=flat-square&logo=apple&logoColor=E8EAF2)](docs/install.md#set-up-macos)
[![Linux](https://img.shields.io/badge/Linux-5_distros-07080F?style=flat-square&logo=linux&logoColor=FFC46B&labelColor=07080F)](docs/install.md#set-up-linux)
[![Windows](https://img.shields.io/badge/Windows-Git_Bash-07080F?style=flat-square&logo=gitforwindows&logoColor=5E8BFF&labelColor=07080F)](docs/install.md#set-up-windows)
[![Claude Code](https://img.shields.io/badge/Claude_Code-A97CFF?style=flat-square&logo=claude&logoColor=white)](docs/ai-agents.md#claude-code)
[![Cursor](https://img.shields.io/badge/Cursor-5E8BFF?style=flat-square&logo=cursor&logoColor=white)](docs/ai-agents.md#cursor)
[![Opencode](https://img.shields.io/badge/Opencode-4FE0C8?style=flat-square&logoColor=07080F)](docs/ai-agents.md#opencode)
[![AGENTS.md](https://img.shields.io/badge/AGENTS.md-ready-FF6FB5?style=flat-square&labelColor=07080F)](AGENTS.md)
[![MIT](https://img.shields.io/badge/license-MIT-FFC46B?style=flat-square&labelColor=07080F)](LICENSE)

[Install](#-install) · [What you get](#-what-you-get) · [Every day](#-every-day) · [Docs](docs/README.md) · [The story](https://stevanpavlovic.com/work/dotfiles)

<sub>New here? [The story page](https://stevanpavlovic.com/work/dotfiles) shows how the skills, agents and guard fit together.</sub>
</div>

## ⚡ Install

Paste one line in a terminal. Do not use a Claude Code window for it.

**macOS** (Terminal app)

```sh
sh -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
```

**Linux** (Debian, Ubuntu, Fedora, Arch, Alpine) and **Windows** (Git Bash)

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/pavstev/dotfiles/main/bootstrap.sh)"
```

A few questions come first: your git name and email, the settings, and the optional looks.
Then it installs. A long step says `still working…` each minute, and the last line is
`dotfiles: done in <time>`. Step by step, with what each OS needs: [docs/install.md](docs/install.md).

## ✨ What you get

| | |
|---|---|
| 🤖 **One config, three agents** | Claude Code, Cursor and Opencode read the same rules, 19 `/ps:` skills and 8 agents from [`claude/`](claude). |
| 🛡️ **Guard hooks** | A Rust guard checks each tool call: risky git, secrets in files, private words, runaway helpers. |
| 🧩 **Presets** | Opt-in working habits: workflow, pace, models, clock, plain writing and more. |
| 🦀 **One command** | `dots` pulls, links and installs. It is idempotent, short, and `dots doctor` checks the result. |
| 🧰 **41 CLI tools** | `rg`, `fd`, `eza`, `bat`, `zoxide`, `fzf`, `atuin`, `delta`, `lazygit` and more, from one [`tools.list`](tools.list) via mise. |
| 🎨 **Optional looks** | Starship prompt, VS Code and Cursor settings, Nerd Font. Off until you pick them. |
| 🔒 **Private by design** | Settings and personal rules stay on the machine. Git hooks block secrets and private words. |
| ✅ **Tested everywhere** | `/os-check` runs the setup on macOS, five Linux distros and a Windows VM. |

## 🧭 Every day

| Command | What it does |
|---|---|
| `dots` | Get the newest setup: pull, link, install what is missing. |
| `dots settings` | Change a setting: git identity, presets, looks, Java, private rules. |
| `dots doctor` | Check the setup. One PASS or FAIL line per check, with the fix. |
| `dots -v` | The same run, with every command shown. |
| `dots uninstall` | Take it all back: links, generated files, merged entries. Your tools and settings stay. |

In Claude Code, Opencode or Cursor, type `/ps:` to see the skills.
Edit files in `~/dotfiles/home/` and `~/dotfiles/claude/`: they are linked, so a change is live at once.

## 📚 Docs

Start with [Install](docs/install.md). Everything else is in the [docs index](docs/README.md): features, AI agents, platforms, customize and uninstall, troubleshooting, and the runbook Claude follows to set up a new machine. How AI agents work on this repo: [AGENTS.md](AGENTS.md).

**Fork it:** change `DOTFILES_REPO` at the top of [`bootstrap.sh`](bootstrap.sh) and the URLs above.

---

<div align="center"><sub>MIT licensed · made by <a href="https://github.com/pavstev">@pavstev</a></sub></div>
