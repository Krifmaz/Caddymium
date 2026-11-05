import { api } from './api.js'

// Global UI state (Svelte 5 runes in a shared module).
export const app = $state({
  me: null,        // /api/me response
  status: null,    // /api/status
  meta: null,      // /api/meta
  toasts: [],
})

let nextToast = 1
const MAX_TOASTS = 5
// Errors stay until closed (they have Copy); others go away by themselves.
// The same message again moves to the newest place with a count instead of
// stacking up.
export function toast(message, kind = 'ok', ms = 4000) {
  const same = app.toasts.find((t) => t.message === message && t.kind === kind)
  if (same) {
    app.toasts = [...app.toasts.filter((t) => t !== same), { ...same, count: (same.count || 1) + 1 }]
    return
  }
  const id = nextToast++
  app.toasts.push({ id, message, kind, count: 1 })
  while (app.toasts.length > MAX_TOASTS) {
    const i = app.toasts.findIndex((t) => t.kind !== 'err')
    app.toasts.splice(i >= 0 ? i : 0, 1)
  }
  if (kind !== 'err') setTimeout(() => closeToast(id), ms)
}
export function closeToast(id) { app.toasts = app.toasts.filter((t) => t.id !== id) }
export function toastErr(e) { toast(e?.message || String(e), 'err') }

export async function refreshStatus() {
  try { app.status = await api.get('/status') } catch { /* shown elsewhere */ }
}
export async function loadMeta() {
  if (!app.meta) app.meta = await api.get('/meta')
  return app.meta
}

export const can = {
  edit: () => ['admin', 'editor'].includes(app.me?.user?.role),
  admin: () => app.me?.user?.role === 'admin',
}
