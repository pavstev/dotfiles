#!/usr/bin/env bash
# map-tree.sh tree [repo]  — print the tracked folders two levels deep, three under a folder of 30+ files
#                            (file counts, top extensions),
#                            the root files and the package.json script names. Facts for writing MAP.md.
# map-tree.sh check [repo] — fail when a path written in backticks in MAP.md does not exist.
set -euo pipefail

cmd="${1:-}"
repo="${2:-.}"
case "$cmd" in tree|check) ;; *) echo "usage: map-tree.sh tree|check [repo]" >&2; exit 2 ;; esac
cd "$repo"
top="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "map-tree.sh: not a git repo: $repo" >&2; exit 1; }
cd "$top"

if [ "$cmd" = "tree" ]; then
  echo "repo: $top"
  echo "tracked files: $(git ls-files | wc -l | tr -d ' ')"
  echo "== folders (path, files, top extensions)"
  git ls-files | awk -F/ '
    NF == 1 { next }
    {
      key = $1 "/"
      if (NF > 2) key2 = $1 "/" $2 "/"; else key2 = ""
      if (NF > 3) { key3 = key2 $3 "/"; par[key3] = key2 } else key3 = ""
      n = split($NF, p, "."); ext = (n > 1) ? p[n] : "-"
      cnt[key]++; e[key SUBSEP ext]++; seen[key SUBSEP ext] = 1
      if (key2 != "") { cnt[key2]++; e[key2 SUBSEP ext]++; seen[key2 SUBSEP ext] = 1 }
      if (key3 != "") { cnt[key3]++; e[key3 SUBSEP ext]++; seen[key3 SUBSEP ext] = 1 }
    }
    END {
      for (k in cnt) {
        if (k in par && cnt[par[k]] < 30) continue
        s = ""
        for (x in seen) { split(x, a, SUBSEP); if (a[1] == k) s = s a[2] "(" e[x] ") " }
        printf "%s\t%d\t%s\n", k, cnt[k], s
      }
    }' | sort | awk -F'\t' '{ printf "%-40s %5d  %s\n", $1, $2, $3 }'
  echo "== root files"
  git ls-files | awk -F/ 'NF == 1' | tr '\n' ' '; echo
  if [ -f package.json ]; then
    echo "== package.json scripts"
    sed -n '/"scripts"[[:space:]]*:/,/}/p' package.json | grep -o '"[A-Za-z0-9:_.-]*"[[:space:]]*:' | sed -E 's/"([^"]*)".*/\1/' | grep -v '^scripts$' | tr '\n' ' '; echo
  fi
  exit 0
fi

# check
[ -f MAP.md ] || { echo "map-tree.sh: no MAP.md in $top" >&2; exit 1; }
bad=0
for t in $(grep -o '`[^` ]*`' MAP.md | tr -d '`' | sort -u); do
  case "$t" in */*|*.*) ;; *) continue ;; esac
  case "$t" in *'*'*|*'<'*|*'>'*|*'$'*|http*|-*|*' '*) continue ;; esac
  p="${t%/}"
  if [ ! -e "$p" ]; then echo "MISSING $t"; bad=1; fi
done
[ "$bad" -eq 0 ] && echo "map ok: every path exists"
exit "$bad"
