# Risk lenses

Six lenses for the fifth reviewer of the panel. Each lens has checks and the evidence that counts.
A finding needs evidence from the diff or the repo: a cited `file:line`, a command output, or a
caller you found. "Looks risky" is not evidence. "I did not find a caller" counts only when you
searched the whole repo and name the command.

## Security and privacy

- Input from outside (request, file, env, URL) reaches a query, a shell, a path or HTML without checks.
- Auth: a route, job or tool that skips the login or role check its neighbours have.
- Secrets, tokens or personal data written to a log, an error text, a URL or a committed file.
- Crypto: a home-made scheme, a weak or fixed key, salt or random source, a compare that is not constant time.
- Session and identity: expiry, rotation, logout, and one user reading another user's data.
- Personal data kept longer or sent to more places than before.

Evidence: the tainted path from source to sink with `file:line` for both; the missing check named
next to the neighbour that has it.

## Migration and data integrity

- Drop, rename or retype of a column or table that code or a job still reads.
- A change that cannot run twice, or has no way back (no down step, no backup, no backfill).
- A statement with no `WHERE`, or one that locks a large table.
- Order: code and schema must work together during the deploy, in both orders.
- New constraints or defaults that old rows break.

Evidence: the statement, plus the readers of the column found with `rg`, plus what happens to
existing rows.

## Dependencies and license

- A new package: name close to a popular one, very new, few downloads, one maintainer.
- A lockfile that changed with no manifest change, or the reverse.
- A major bump, or a version range opened wider than before.
- Install scripts, git or URL sources, packages pulled from a non-default registry.
- A license that the repo cannot use (copyleft in a closed product).

Evidence: the manifest line, the registry page or `npm view` output, the lockfile entry.

## Ops: infra, CI, rollout, rollback

- A workflow, image or Terraform change that widens permissions, opens a port or pins nothing.
- Secrets passed to a step that runs untrusted code (pull request from a fork).
- No health check, timeout or limit on something new.
- Rollout order: what breaks while old and new versions run side by side.
- Rollback: can the change be undone without data loss?

Evidence: the file and line, plus the permission or order problem in one sentence.

## API contract

- A field, status code or error shape removed or renamed that a client may use.
- A required input added, or a type narrowed, with no version bump.
- The schema file (openapi, proto) and the code disagree.
- Pagination, ordering or default values changed.
- Callers in the repo and docs not updated.

Evidence: the old and new shape side by side, plus the callers found with `rg`.

## Frontend: accessibility, performance, UI state

- Controls with no label, no keyboard path, or no visible focus.
- Colour contrast or text size below the usual minimum; the layout at 320 px width.
- Loading, empty and error states missing for a new data fetch.
- A large new dependency or an unbounded list rendered at once.
- State that is lost on reload or leaks between users or routes.

Evidence: the element or style with `file:line`; a browser run when the pane is available.
