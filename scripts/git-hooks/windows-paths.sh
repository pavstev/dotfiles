#!/usr/bin/env bash
# windows_bad_paths: read paths on stdin, print those Windows cannot hold.
# A bad path makes `git clone` fail on Windows ("invalid path") and leaves a
# half checkout. Rejected: one of  : < > " | ? * \  in a name, a name that ends
# in a dot or space, and a device name (con, prn, aux, nul, com1-9, lpt1-9).
# Used by pre-commit and scripts/test-dotfiles.sh.
windows_bad_paths() {
  grep -iE '[:<>"|?*\\]|[. ](/|$)|(^|/)(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.[^/]*)?(/|$)' || true
}
