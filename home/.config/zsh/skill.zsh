# ============================================================================
#  skill — manage Claude/Cursor/Opencode skills backed by the dotfiles repo
#  Source: $DOTFILES/home/.config/zsh/skill.zsh  (symlinked from this repo)
#  Edits here are live immediately — no apply step.
# ============================================================================

# ----- internal: paths -------------------------------------------------------

_skill_repo() { printf '%s' "${DOTFILES:-$HOME/dotfiles}"; }
_skill_src()  { printf '%s' "$(_skill_repo)/claude/skills"; }

_skill_sync_links() {
  local sync="$(_skill_repo)/claude/skill-sync.sh"
  [[ -x "$sync" ]] && "$sync" >/dev/null 2>&1
}

# ----- internal: copper/stone color theme ------------------------------------

_sk_h()   { [[ -t 1 ]] && printf '\e[38;5;209m%s\e[0m' "$*" || printf '%s' "$*"; }  # copper
_sk_ok()  { [[ -t 1 ]] && printf '\e[38;5;71m%s\e[0m'  "$*" || printf '%s' "$*"; }  # green
_sk_dim() { [[ -t 1 ]] && printf '\e[38;5;245m%s\e[0m' "$*" || printf '%s' "$*"; }  # stone
_sk_err() { [[ -t 1 ]] && printf '\e[38;5;167m%s\e[0m' "$*" || printf '%s' "$*"; }  # red

# ----- internal: SKILL.md field readers --------------------------------------

_skill_name_from_md() {
  local md="$1"
  [[ -f "$md" ]] || return 1
  grep -m1 '^name:' "$md" | sed 's/^name:[[:space:]]*//' | tr -d '"' | xargs
}

_skill_desc_from_md() {
  local md="$1"
  [[ -f "$md" ]] || return 1
  grep -m1 '^description:' "$md" | sed 's/^description:[[:space:]]*//' | tr -d '"' | cut -c1-65
}

# ----- skill install ---------------------------------------------------------

_skill_install() {
  local arg="${1:-}"

  if [[ -z "$arg" ]]; then
    printf '%s skill install <name|owner/repo|url>\n' "$(_sk_err 'usage:')" >&2
    return 1
  fi

  # Direct: full URL
  if [[ "$arg" == https://* || "$arg" == git@* ]]; then
    _skill_install_from_url "$arg" "$(basename "$arg" .git)"
    return
  fi

  # Direct: owner/repo shorthand
  if [[ "$arg" == */* ]]; then
    _skill_install_from_url "https://github.com/$arg" "$(basename "$arg")"
    return
  fi

  # Search GitHub by name + topic ai-skill
  local name="$arg"

  if ! command -v gh >/dev/null 2>&1; then
    printf '%s gh CLI not found. Install: brew install gh\n' "$(_sk_err 'error:')" >&2
    return 1
  fi
  if ! command -v jq >/dev/null 2>&1; then
    printf '%s jq not found. Install: brew install jq\n' "$(_sk_err 'error:')" >&2
    return 1
  fi

  printf 'Searching GitHub for %s skills tagged %s...\n' \
    "$(_sk_h "$name")" "$(_sk_dim 'ai-skill')"

  local results
  results="$(gh search repos "$name" --topic ai-skill \
    --json fullName,description,url --limit 8 2>/dev/null)" || {
    printf '%s GitHub search failed. Run: gh auth status\n' "$(_sk_err 'error:')" >&2
    return 1
  }

  local count
  count="$(printf '%s' "$results" | jq 'length')"

  if [[ "$count" -eq 0 ]]; then
    printf '%s no results for "%s" with topic ai-skill\n' "$(_sk_err 'not found:')" "$name" >&2
    printf '%s  direct install: %s\n' \
      "$(_sk_dim 'tip:')" "$(_sk_dim 'skill install owner/repo')" >&2
    return 1
  fi

  echo
  printf '%s' "$results" | jq -r 'to_entries[] | "\(.key+1)) \(.value.fullName)  — \(.value.description // "")"' \
    | while IFS= read -r line; do
        printf '  %s\n' "$line"
      done
  echo

  local choice=1
  if [[ "$count" -gt 1 ]]; then
    printf 'Pick [1-%d] or q to cancel: ' "$count"
    read -r choice
    [[ "$choice" =~ ^[0-9]+$ && "$choice" -ge 1 && "$choice" -le "$count" ]] \
      || { printf 'Cancelled.\n'; return 0; }
  else
    local repo_name="$(printf '%s' "$results" | jq -r '.[0].fullName')"
    printf 'Install %s? [Y/n] ' "$(_sk_h "$repo_name")"
    read -r reply
    [[ -z "$reply" || "$reply" =~ ^[Yy]$ ]] || { printf 'Cancelled.\n'; return 0; }
  fi

  local repo_full="$(printf '%s' "$results" | jq -r ".[$((choice-1))].fullName")"
  local repo_url
  repo_url="https://github.com/$repo_full"
  _skill_install_from_url "$repo_url" "$name"
}

# Download a skill from a git URL and add it to the dotfiles repo.
# $1 = clone URL   $2 = hint name (used if we can't infer one)
_skill_install_from_url() {
  local repo_url="$1" hint_name="${2:-}"
  local src; src="$(_skill_src)"

  local tmpdir
  tmpdir="$(mktemp -d)"

  printf 'Cloning %s...\n' "$(_sk_dim "$repo_url")"
  if ! git clone --depth 1 --quiet "$repo_url" "$tmpdir/repo" 2>/dev/null; then
    printf '%s failed to clone %s\n' "$(_sk_err 'error:')" "$repo_url" >&2
    rm -rf "$tmpdir"
    return 1
  fi

  local target_dir skill_name

  # --- auto-detect skill shape ---
  # 1. Root SKILL.md → single-skill repo
  if [[ -f "$tmpdir/repo/SKILL.md" ]]; then
    skill_name="$(_skill_name_from_md "$tmpdir/repo/SKILL.md")"
    skill_name="${skill_name:-$hint_name}"
    target_dir="$tmpdir/repo"
    printf 'Single-skill repo → installing as %s\n' "$(_sk_h "$skill_name")"

  # 2. Subfolder named after the hint — search depth 2 (flat) and depth 3 (e.g. skills/<name>/)
  elif [[ -n "$hint_name" ]]; then
    local found_dir=""
    [[ -f "$tmpdir/repo/$hint_name/SKILL.md" ]] && found_dir="$tmpdir/repo/$hint_name"
    if [[ -z "$found_dir" ]]; then
      found_dir="$(command find "$tmpdir/repo" -maxdepth 3 -type d -name "$hint_name" 2>/dev/null \
                   | while IFS= read -r d; do [[ -f "$d/SKILL.md" ]] && echo "$d" && break; done | head -1)"
    fi
    if [[ -n "$found_dir" ]]; then
      skill_name="$hint_name"
      target_dir="$found_dir"
      printf 'Multi-skill repo — using %s/ subfolder\n' "$(_sk_h "$skill_name")"
    else
      false  # fall through to else
    fi
  fi
  if [[ -z "${target_dir:-}" ]]; then
    # 3. Collect all SKILL.md subfolders and let user choose
    local -a candidates=()
    while IFS= read -r d; do
      [[ "$(basename "$d")" != _* ]] && candidates+=("$d")
    done < <(command find "$tmpdir/repo" -maxdepth 3 -name SKILL.md \
               ! -path "$tmpdir/repo/SKILL.md" -exec dirname {} \; | sort)

    if [[ ${#candidates[@]} -eq 0 ]]; then
      printf '%s no SKILL.md found in repo root or subfolders\n' "$(_sk_err 'error:')" >&2
      rm -rf "$tmpdir"; return 1
    fi

    echo "Skills available in this repo:"
    local i=1 cname="" d=""
    for c in "${candidates[@]}"; do
      cname="$(basename "$c")"
      d="$(_skill_desc_from_md "$c/SKILL.md")"
      printf '  %d) %-20s  %s\n' "$i" "$cname" "$(_sk_dim "$d")"
      ((i++))
    done
    printf 'Which skill? [1-%d]: ' "${#candidates[@]}"
    read -r choice
    [[ "$choice" =~ ^[0-9]+$ && "$choice" -ge 1 && "$choice" -le "${#candidates[@]}" ]] \
      || { printf 'Cancelled.\n'; rm -rf "$tmpdir"; return 0; }
    # zsh arrays are 1-indexed; choice is already 1-based from the menu
    skill_name="$(basename "${candidates[$choice]}")"
    target_dir="${candidates[$choice]}"
  fi

  # --- safety: abort if skill_name is empty (prevents rm -rf $src) ---
  if [[ -z "$skill_name" || -z "$target_dir" ]]; then
    printf '%s could not determine skill name — aborting\n' "$(_sk_err 'error:')" >&2
    rm -rf "$tmpdir"; return 1
  fi

  # --- guard: already installed? ---
  local dest="$src/$skill_name"
  if [[ -d "$dest" ]]; then
    printf '%s already installed. Update it? [y/N] ' "$(_sk_h "$skill_name")"
    read -r reply
    [[ "$reply" =~ ^[Yy]$ ]] || { rm -rf "$tmpdir"; return 0; }
    rm -rf "$dest"
  fi

  cp -r "$target_dir" "$dest"
  rm -rf "$dest/.git" "$dest/.github" "$dest/.gitignore"

  _skill_sync_links

  local repo="$(_skill_repo)"
  (
    cd "$repo" || exit 1
    git add -- "claude/skills/$skill_name"
    if git diff --cached --quiet; then
      printf '%s %s — already up to date\n' "$(_sk_ok '✓')" "$(_sk_h "$skill_name")"
    elif git commit -m "skill: add $skill_name" --quiet; then
      printf '%s committed\n' "$(_sk_ok '✓')"
      if git push --quiet 2>/dev/null; then
        printf '%s pushed — all machines sync on next pull\n' "$(_sk_ok '✓')"
      else
        printf '%s push failed — run: cd %s && git push\n' \
          "$(_sk_err '!')" "$repo" >&2
      fi
    else
      printf '%s commit failed\n' "$(_sk_err '!')" >&2
    fi
  )

  rm -rf "$tmpdir"
  printf '\n%s %s installed\n' "$(_sk_ok '✓')" "$(_sk_h "$skill_name")"
}

# ----- skill list ------------------------------------------------------------

_skill_list() {
  local src="$(_skill_src)"
  local active_root="$HOME/.claude/skills"
  local any=0

  printf '\n%s\n\n' "$(_sk_h 'Installed skills:')"
  for d in "$src"/*/; do
    [[ -d "$d" ]] || continue
    local name="$(basename "$d")"
    [[ "$name" == _* || "$name" == .* ]] && continue
    [[ -f "$d/SKILL.md" ]] || continue
    any=1

    local desc="$(_skill_desc_from_md "$d/SKILL.md")"
    local badge=""
    if [[ -L "$active_root/$name" || -d "$active_root/$name" ]]; then
      badge="$(_sk_ok '✓ active')"
    else
      badge="$(_sk_dim '○ unlinked')"
    fi

    printf '  %-28s  %s  %s\n' \
      "$(_sk_h "$name")" "$badge" "$(_sk_dim "${desc:0:55}")"
  done

  if [[ $any -eq 0 ]]; then
    printf '  %s\n' "$(_sk_dim 'No skills yet.  Try: skill install <name>')"
  fi
  echo
}

# ----- skill remove ----------------------------------------------------------

_skill_remove() {
  local name="${1:-}"
  if [[ -z "$name" ]]; then
    printf '%s skill remove <name>\n' "$(_sk_err 'usage:')" >&2; return 1
  fi

  local src="$(_skill_src)"
  if [[ ! -d "$src/$name" ]]; then
    printf '%s skill "%s" not found in %s\n' "$(_sk_err 'error:')" "$name" "$src" >&2
    return 1
  fi

  printf 'Remove %s from repo and all machines? [y/N] ' "$(_sk_h "$name")"
  read -r reply
  [[ "$reply" =~ ^[Yy]$ ]] || { printf 'Cancelled.\n'; return 0; }

  local repo="$(_skill_repo)"

  # Determine whether the skill is tracked in git (vs just untracked on disk)
  local is_tracked=0
  (cd "$repo" && git ls-files -- "claude/skills/$name" 2>/dev/null | grep -q .) && is_tracked=1

  if [[ $is_tracked -eq 1 ]]; then
    (
      cd "$repo" || exit 1
      if ! git rm -r --quiet -- "claude/skills/$name" 2>/dev/null; then
        printf '%s git rm failed\n' "$(_sk_err '!')" >&2; exit 1
      fi
      if git commit -m "skill: remove $name" --quiet; then
        printf '%s committed\n' "$(_sk_ok '✓')"
        if git push --quiet 2>/dev/null; then
          printf '%s pushed — skill removed from all machines on next sync\n' "$(_sk_ok '✓')"
        else
          printf '%s push failed — run: cd %s && git push\n' "$(_sk_err '!')" "$repo" >&2
        fi
      else
        printf '%s nothing to commit\n' "$(_sk_dim '○')"
      fi
    ) || { printf '%s remove failed\n' "$(_sk_err '!')" >&2; return 1; }
  else
    rm -rf "$src/$name"
    printf '%s removed locally (was not yet committed — no git push needed)\n' "$(_sk_dim '○')"
  fi

  _skill_sync_links
  printf '\n%s %s removed\n' "$(_sk_ok '✓')" "$(_sk_h "$name")"
}

# ----- skill create ----------------------------------------------------------

_skill_create() {
  local name="${1:-}"
  if [[ -z "$name" ]]; then
    printf '%s skill create <name>\n' "$(_sk_err 'usage:')" >&2; return 1
  fi

  local src="$(_skill_src)"
  local dest="$src/$name"

  if [[ -d "$dest" ]]; then
    printf '%s skill "%s" already exists at %s\n' "$(_sk_err 'error:')" "$name" "$dest" >&2
    return 1
  fi
  if [[ ! -d "$src/_template" ]]; then
    printf '%s _template not found at %s/_template\n' "$(_sk_err 'error:')" "$src" >&2
    return 1
  fi

  cp -r "$src/_template" "$dest"
  # Pre-fill the name field
  # (no `sed -i`: its flags differ between macOS and Linux)
  if sed "s/^name: .*/name: $name/" "$dest/SKILL.md" >"$dest/SKILL.md.tmp" 2>/dev/null; then
    mv "$dest/SKILL.md.tmp" "$dest/SKILL.md"
  else
    rm -f "$dest/SKILL.md.tmp"
  fi

  printf '%s scaffolded %s\n' "$(_sk_ok '✓')" "$(_sk_h "$name")"
  printf '  edit: %s\n\n' "$(_sk_dim "$dest/SKILL.md")"

  if [[ -n "${EDITOR:-}" ]]; then
    "${EDITOR}" "$dest/SKILL.md"
  fi
}

# ----- skill status ----------------------------------------------------------

_skill_status() {
  local repo="$(_skill_repo)"
  local src="$(_skill_src)"
  local active_root="$HOME/.claude/skills"

  printf '\n%s\n' "$(_sk_h '── skill status ──────────────────────────────')"

  # 1. Git state
  printf '\n%s\n' "$(_sk_h 'Repo git state:')"
  (
    cd "$repo" || { printf '  cannot cd to %s\n' "$repo" >&2; exit 1; }
    local dirty="$(git status --short -- claude/skills 2>/dev/null)"
    if [[ -z "$dirty" ]]; then
      printf '  %s claude/skills is clean\n' "$(_sk_ok '✓')"
    else
      printf '%s\n' "$dirty" | while IFS= read -r line; do
        printf '  %s\n' "$line"
      done
    fi
    local ahead="$(git rev-list --count origin/main..HEAD 2>/dev/null || printf '?')"
    local behind="$(git rev-list --count HEAD..origin/main 2>/dev/null || printf '?')"
    printf '  %s\n' "$(_sk_dim "origin/main: ${ahead} ahead, ${behind} behind")"
  )

  # 2. Symlink state
  printf '\n%s\n' "$(_sk_h 'Local symlink state (→ ~/.claude/skills):')"
  local any=0
  for d in "$src"/*/; do
    [[ -d "$d" ]] || continue
    local name="$(basename "$d")"
    [[ "$name" == _* || "$name" == .* ]] && continue
    [[ -f "$d/SKILL.md" ]] || continue
    any=1
    if [[ -L "$active_root/$name" || -d "$active_root/$name" ]]; then
      printf '  %s %s\n' "$(_sk_ok '✓')" "$name"
    else
      printf '  %s %s\n' "$(_sk_dim '○')" "$(_sk_dim "$name (not linked)")"
    fi
  done
  [[ $any -eq 0 ]] && printf '  %s\n' "$(_sk_dim 'No skills in repo')"

  echo
}

# ----- skill help ------------------------------------------------------------

_skill_help() {
  printf '\n%s  manage Claude/Cursor/Opencode skills\n\n' "$(_sk_h 'skill')"
  printf '  %s  %-24s  %s\n' "$(_sk_h 'skill')" "$(_sk_dim 'install <name>')"     "install from GitHub (searches topic: ai-skill)"
  printf '  %s  %-24s  %s\n' "$(_sk_h 'skill')" "$(_sk_dim 'install owner/repo')" "install directly from a GitHub repo"
  printf '  %s  %-24s  %s\n' "$(_sk_h 'skill')" "$(_sk_dim 'list')"              "list installed skills and active status"
  printf '  %s  %-24s  %s\n' "$(_sk_h 'skill')" "$(_sk_dim 'remove <name>')"     "delete skill from repo (syncs to all machines)"
  printf '  %s  %-24s  %s\n' "$(_sk_h 'skill')" "$(_sk_dim 'create <name>')"     "scaffold a new skill from _template"
  printf '  %s  %-24s  %s\n' "$(_sk_h 'skill')" "$(_sk_dim 'status')"            "git state and symlink state"
  echo
}

# ----- entry point -----------------------------------------------------------

skill() {
  local cmd="${1:-help}"
  shift 2>/dev/null || true
  case $cmd in
    install)                _skill_install  "$@" ;;
    list|ls)                _skill_list     "$@" ;;
    remove|rm|delete)       _skill_remove   "$@" ;;
    create|new)             _skill_create   "$@" ;;
    status|st)              _skill_status   "$@" ;;
    help|-h|--help|*)       _skill_help          ;;
  esac
}
