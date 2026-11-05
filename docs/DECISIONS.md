# Decisions

Non-obvious calls, one line each, newest last.

- 2026-11-04 · Releases moved to the kit's shared workflows (kit-release + kit-docker, pinned to kit v0.2.0) first, so
  every merge to main ships a normal vX.Y.Z release from the start; the old tag-triggered release job is gone.
- 2026-11-04 · The panel image stays amd64-only per release (arm64 on request), as before: arm64 under emulation
  costs most of an hour of Actions minutes on a private repo.
- 2026-11-04 · Version tags are now created by the release (GITHUB_TOKEN), which does not trigger other workflows, so
  the Caddy image keeps publishing `:latest` on caddy/ changes but no longer gets per-version tags.
