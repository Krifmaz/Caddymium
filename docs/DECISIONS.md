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
