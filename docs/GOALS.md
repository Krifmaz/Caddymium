# Goals

The owner's goals for Caddymium, in their own words. Work on them hands-off; non-obvious calls are logged in
[DECISIONS.md](DECISIONS.md).

## Make Caddymium never leave my sites down, then adopt the kit

Fix first (each with tests):

1. Safe updates: if any step of a Caddy update fails after Caddy was asked to stop (e.g. "ask Caddy to stop: admin
   API request timed out"), kill a hung process, start the previous binary, confirm the admin API and the :80/:443
   listeners answer, and notify. An update only counts as done when the new Caddy is serving; otherwise roll back
   automatically.
2. Restart that works: after "Restarting Caddy…" verify Caddy is up within 30s; if not, retry once, then roll back
   to the last good binary and notify. Never needs a container restart to recover.
3. Watchdog: if the admin API or the listeners stop answering for 60s while Caddy should be running, restart Caddy
   (last good binary) and notify. Show it in the sidebar ("Caddy restarted by watchdog at …").
4. Clear the "automatic update failed" banner once a later update succeeds or the running version is already the
   latest.
5. Updates never change site config: new Security defaults (bot blocking, robots.txt, headers) must never switch on
   for existing sites through an update; offer them as a suggestion card the user can apply per site or for all.
6. Clear errors with Copy for every failure, saying what happened and what to do.

Then adopt the kit: Encore look, What's new, toasts with Copy, bug reports, diagnostics, settings from a schema,
shared release workflows.

## Shared app block

Build on Krifmaz/kit (pin the latest kit tag; read its docs/DESIGN.md and CLAUDE.md first). Use the kit for the look
(Encore theme), What's new and changelog, self-updating web app, notice banners, smart toasts with Copy, confirm
dialogs, bug reports and crash logs, diagnostics, settings from a schema, offline mode and the shared release
workflows. Don't rebuild what the kit has; if something is missing or generic, add it to the kit instead of the app.

Work hands-off: make your own calls (log non-obvious ones in docs/DECISIONS.md), one PR per change, fix CI until
green, squash-merge yourself, delete the branch. Every merge ships a normal vX.Y.Z release (no pre-releases). Never
skip tests or force-push main. Only stop to ask about deleting user data, a needed secret or paid service. One short
summary per finished goal.
