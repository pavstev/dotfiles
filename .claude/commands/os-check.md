---
description: Manual check of ./dotfiles on macOS, Linux and Windows before a commit. Report only.
argument-hint: "[macos|linux|windows|all] [--force] [--since REF] [--host NAME | --local]"
---

Run the manual OS check through one helper. Show only its short report.

1. Start ONE helper: the Agent tool with `subagent_type: worker`. Give it this brief, with the
   arguments `$ARGUMENTS` filled in:
   - Run `bash scripts/os-check.sh $ARGUMENTS` from the repo root, in the background, and wait
     for it to end. Linux can take 20 to 40 minutes. Deadline: 60 minutes.
   - For each OS that failed, read `.os-check/<os>.log` and find the likely cause.
   - Do not edit, stage or commit any file. Report only.
   - Hand back at most 1,500 characters: per OS PASS, FAIL or SKIP; for each FAIL the 3 most
     useful log lines and one suggested fix (file and change).
2. Show the helper's report as it is. Do not read or paste the full logs.
3. On FAIL, do not commit the change unless the user says so.
