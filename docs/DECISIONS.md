# Decisions

Non-obvious calls, one line each, newest last.

- 2026-11-04 · Releases moved to the kit's shared workflows (kit-release + kit-docker, pinned to kit v0.2.0) first, so
  every merge to main ships a normal vX.Y.Z release from the start; the old tag-triggered release job is gone.
- 2026-11-04 · The panel image stays amd64-only per release (arm64 on request), as before: arm64 under emulation
  costs most of an hour of Actions minutes on a private repo.
- 2026-11-04 · Version tags are now created by the release (GITHUB_TOKEN), which does not trigger other workflows, so
  the Caddy image keeps publishing `:latest` on caddy/ changes but no longer gets per-version tags.
- 2026-11-05 · Goals 1 (safe updates) and 2 (verified restart) ship as one PR: they are the same restart routine
  (`recovery.rs`), used by updates, rollback, reset, the new Restart button and later the watchdog.
- 2026-11-05 · "Serving" means the admin API answers and every TCP port the running config's HTTP servers listen on
  accepts connections at the probe host (plus `CADDYMIUM_PROBE`); a config with no HTTP servers (fresh install) only
  needs the admin API.
- 2026-11-05 · Fallback order after a failed restart: undo the operation (the previous build), then the image's
  built-in Caddy. A process the panel killed is never set aside as "broken" by the supervisor/entrypoint; the panel
  picks the fallback itself.
- 2026-11-05 · Two-container layout: the panel kills a hung Caddy through `caddy.kill` in the shared bin volume,
  watched by the caddy image's entrypoint (`caddy.gen` counts starts). An older caddy image without this gets a
  clear "restart the Caddy container" message instead.
- 2026-11-05 · The watchdog checks every 5s and only counts while Caddy should run (detected, no plugin job or
  restart in progress). Its restart is a normal job on the Plugins page ("watchdog") so its log is visible; the
  sidebar shows the last one for 24h. `CADDYMIUM_WATCHDOG=false` turns it off.
- 2026-11-05 · Recommended protections are a registry (`suggestions.rs`). On an install that already has sites, a
  recommendation it has not seen is pinned off in the stored defaults and offered on the Security page instead.
  Installs that got 0.6.0's bot blocking, privacy and CrowdSec defaults without ever saving the Security page get
  their sites back as before 0.6.0 (with the three as suggestions); a saved Security page is kept as chosen.
- 2026-11-05 · Applying a suggestion edits the model like any other change: it goes live through Review & apply.
- 2026-11-05 · Copyable errors (goal 6) are built in the Svelte UI for now (error toasts stay until closed with Copy and
  merge duplicates; every error box has Copy; server errors say what to do). The kit's web components are Leptos, so
  they arrive with the kit adoption, which replaces these.
- 2026-11-05 · Kit pinned at v0.2.1 (latest tag). rusqlite goes 0.40 → 0.39 so the panel and the kit link one
  sqlite3 (the panel's code needed no change).
- 2026-11-05 · The kit's tables live in `kit.db` next to `caddymium.db`, so the panel's backups, restores and
  migrations never touch them. Losing it loses only bug reports, crash logs and What's new "seen" marks.
- 2026-11-05 · The kit's routes use the panel's sign-in (sessions, API keys, CSRF, 2FA). Panel admins are the kit's
  admins and owner; editors and viewers are members. The kit's `/api/health` replaces the panel's own.
- 2026-11-05 · Notices: Caddy not serving (every 30s, quiet while a plugin job runs), a failed automatic update
  (every 5 min) and the data folder. Diagnostics: admin API, listeners, data folder, database, Caddy log folder.
  A new bug report notifies through the panel's channels (event `bug_report`).
- 2026-11-05 · docs/releases/0.7.0.md makes the next release 0.7.0 (the reliability work plus the kit's server
  side); the notes say the kit's web app follows, so they never promise what isn't shipped.
- 2026-11-05 · The web app moves to the kit (Leptos, `web/`, built by trunk) page by page. Until every page is
  there it is served at `/next/` next to the classic Svelte UI at `/`, with "Try the new look" in the classic
  sidebar; a page that hasn't moved links to its classic version. The last step makes it the default and removes
  the Svelte UI.
- 2026-11-05 · The kit has only a few icons, so the web app adds the Lucide icons it needs (`web/src/icons.rs`) in the
  kit's style; they move into the kit when the kit takes app icons.
- 2026-11-05 · Themes: the kit's four (Encore default, Dark, Black, Light) plus System, per browser. The 2FA QR code
  comes from the server as SVG, so the web app needs no QR library.
- 2026-11-05 · Every panel answer carries `x-caddymium: 1`, so the kit web app tells a proxy's error page (the
  panel restarting) from the panel's own errors.
- 2026-11-05 · Settings from a schema: backend/settings.toml describes Caddy's global options for kit-server-settings, and
  settings_form.rs maps each flat key onto the panel's one settings document, so the generator, backups and the
  DDNS, updates and Security pages keep reading the same values and the same checks apply. The kit routes live at
  /api/settings/form* because /api/settings is the whole document. No key has an environment variable: these are
  panel data, not deployment config. Lists are one-per-line text fields; DDNS, updates and Security keep their
  own pages (rows and live status the schema can't describe).
- 2026-11-05 · Cutover: the kit web app is the panel's only UI. trunk builds it into backend/ui/dist with
  public_url `/`, the backend serves it at `/` (hashed files cached for good, index.html no-cache, unknown non-/api
  paths get index.html) and the Svelte UI (frontend/) is gone, so nothing in the build needs Node. `/next` and
  `/next/*` answer a permanent redirect to `/`; the browser keeps the hash, so `/next/#/sites` lands on `/#/sites`.
