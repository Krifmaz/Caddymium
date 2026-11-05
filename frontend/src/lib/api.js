// Thin JSON client for the Caddymium API. The CSRF token comes from /api/me
// (or login) and is sent on every state-changing request.
let csrf = ''
export function setCSRF(t) { csrf = t || '' }

export class APIError extends Error {
  constructor(status, message) { super(message); this.status = status }
}

let onUnauthorized = () => {}
export function handleUnauthorized(fn) { onUnauthorized = fn }

async function req(method, path, body) {
  const opts = { method, headers: {}, credentials: 'same-origin' }
  if (body !== undefined) {
    opts.headers['Content-Type'] = 'application/json'
    opts.body = JSON.stringify(body)
  }
  if (method !== 'GET') opts.headers['X-CSRF-Token'] = csrf
  let res
  try {
    res = await fetch('/api' + path, opts)
  } catch (e) {
    throw new APIError(0, `Can't reach Caddymium (${e.message || 'network error'}). Check that the container is running and this device is online, then try again.`)
  }
  const text = await res.text()
  let data = null
  try { data = text ? JSON.parse(text) : null } catch { data = text }
  if (!res.ok) {
    let msg = (data && data.error) || res.statusText || 'request failed'
    if (res.status >= 500 && !(data && data.error)) {
      msg = `Caddymium answered HTTP ${res.status} ${res.statusText}. It may be restarting: wait a moment and try again; if it keeps happening, copy this and check the panel's log (docker logs caddymium).`
    }
    if (res.status === 401 && path !== '/login' && path !== '/login/totp') onUnauthorized()
    throw new APIError(res.status, msg)
  }
  return data
}

export const api = {
  get: (p) => req('GET', p),
  post: (p, b) => req('POST', p, b ?? {}),
  put: (p, b) => req('PUT', p, b),
  del: (p) => req('DELETE', p),
}
