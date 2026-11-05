<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { onMount } from 'svelte'
  import {
    Radar, ArrowLeft, ArrowRight, CopyPlus, Trash2, Sparkles, Server, Folder, CornerUpRight, MessageSquare, Code, Activity,
    Route, ShieldCheck, Lock, Gauge, Construction, ChevronDown, Plus, Check, Info, CircleAlert, Fingerprint, ArrowLeftRight, TriangleAlert,
  } from '@lucide/svelte'
  import { api } from '../lib/api.js'
  import { app, toast, toastErr, refreshStatus, loadMeta, can } from '../lib/state.svelte.js'
  import { go } from '../lib/router.svelte.js'
  import { clone } from '../lib/util.js'
  import ListInput from '../lib/ListInput.svelte'
  import KVInput from '../lib/KVInput.svelte'
  import Switch from '../lib/Switch.svelte'
  import Avatar from '../lib/Avatar.svelte'
  import HostInput from '../lib/HostInput.svelte'
  import ScanModal from '../lib/ScanModal.svelte'
  import { knownDomains } from '../lib/domains.js'
  import { templates } from '../lib/templates.js'

  let { id } = $props()
  const isNew = $derived(id === 'new')

  function blank() {
    return {
      name: '', hosts: [''], kind: 'proxy', enabled: true, tags: [],
      config: { proxy: { upstreams: [''] }, tls: { mode: 'auto' }, access: {}, routes: [] },
    }
  }

  let site = $state(null)
  let tagText = $state('')
  let snippets = $state([])
  let busy = $state(false)
  let error = $state('')
  let rawCheck = $state(null)
  let accessLog = $state('default')
  let domainList = $state([])
  let defaultDomain = $state('')
  let hostKey = $state(0)
  let scanning = $state(false)
  let portsText = $state('')
  let secDefaults = $state(null)   // Security page defaults (settings.security.defaults)
  const appPresets = [['vaultwarden', 'Vaultwarden / Bitwarden apps'], ['jellyfin', 'Jellyfin apps'], ['plex', 'Plex apps'], ['immich', 'Immich apps']]
  const shieldRows = [['crowdsec', 'Block known attackers (CrowdSec)'], ['bots', 'Block AI crawlers and scrapers'], ['privacy', 'Keep out of search engines'], ['rate_limit', 'Limit requests per visitor'], ['home_only', 'Home only']]
  const onOff = (v) => (v ? 'on' : 'off')
  function triValue(k) { const v = site.config.access.shields[k]; return v === true ? 'on' : v === false ? 'off' : '__default' }
  function setTri(k, v) { site.config.access.shields[k] = v === 'on' ? true : v === 'off' ? false : null }
  const loginNames = { '': 'no login wall', sso: 'sign-in + 2FA', basic: 'shared password' }
  const sh = $derived(site?.config?.access?.shields)
  const login = $derived(sh ? (sh.login ?? secDefaults?.login ?? '') : '')

  // Quick fill from a scan result: upstream, and name/hostname if still empty.
  function usePick(r) {
    scanning = false
    const p = site.config.proxy
    const ups = p.upstreams.filter((u) => u.trim())
    p.upstreams = ups.length > 1 ? [...ups, r.upstream] : [r.upstream]
    if (r.scheme === 'https') p.tls_skip_verify = true
    const label = (r.guess || r.host || '').trim()
    const slug = label.toLowerCase().split(/[^a-z0-9]+/).find((w) => w.length > 1) || ''
    if (!site.name && label) site.name = label
    const empty = site.hosts.every((h) => !h.trim() || domainList.includes(h.trim()))
    if (empty && slug && defaultDomain && !/^\d/.test(slug)) {
      site.hosts = [`${slug}.${defaultDomain}`]
      hostKey++
    }
    toast(`Using ${r.upstream}${r.scheme === 'https' ? ' (self-signed certificates allowed)' : ''}`)
  }

  const feat = (fid) => (app.meta?.features || []).find((f) => f.id === fid)
  const available = (fid) => feat(fid)?.available !== false

  function ensure(s) {
    const c = s.config
    c.tls ||= { mode: 'auto' }
    c.tls.mode ||= 'auto'
    c.access ||= {}
    c.routes ||= []
    c.proxy ||= { upstreams: [''] }
    c.proxy.upstreams = c.proxy.upstreams?.length ? c.proxy.upstreams : ['']
    c.static ||= { root: '' }
    c.redirect ||= { to: '', code: 302 }
    c.respond ||= { status: 200, body: '' }
    c.forward_proxy ||= {}
    c.forward_proxy.basic_auth ||= []
    c.forward_proxy.ports ||= []
    c.forward_proxy.acl ||= []
    c.access.allow_cidrs ||= []
    c.access.deny_cidrs ||= []
    c.access.basic_auth ||= []
    c.access.shields ||= {}
    c.access.shields.app_paths ||= []
    c.access.shields.sso_users ||= []
    if (c.access.crowdsec && c.access.shields.crowdsec == null) c.access.shields.crowdsec = true
    c.headers_remove ||= []
    c.compression ||= []
    c.snippets ||= []
    c.www_redirect ||= ''
    c.proxy.lb_policy ||= ''
    if (!s.hosts?.length) s.hosts = ['']
    return s
  }

  onMount(async () => {
    loadMeta().catch(() => {})
    api.get('/snippets').then((x) => (snippets = x)).catch(() => {})
    try {
      const [s, doms, all, st] = await Promise.all([
        isNew ? blank() : api.get('/sites/' + id),
        api.get('/domains').catch(() => []),
        api.get('/sites').catch(() => []),
        api.get('/settings').catch(() => ({})),
      ])
      domainList = knownDomains(doms, all, st.default_domain)
      secDefaults = st.security?.defaults || null
      defaultDomain = st.default_domain || domainList[0] || ''
      site = ensure(s)
      tagText = (site.tags || []).join(', ')
      portsText = site.config.forward_proxy.ports.join(', ')
      accessLog = site.config.access_log == null ? 'default' : site.config.access_log ? 'on' : 'off'
    } catch (e) { toastErr(e); go('/sites') }
  })

  function applyTemplate(t) {
    if (!t) return
    const s = ensure(clone({ ...blank(), ...t.site, config: { ...blank().config, ...t.site.config } }))
    const typed = site.hosts.some((h) => h.trim() && !domainList.includes(h.trim()))
    s.hosts = typed ? site.hosts : [defaultDomain ? `${t.id}.${defaultDomain}` : '']
    s.name = site.name || t.name
    site = s
    portsText = site.config.forward_proxy.ports.join(', ')
    hostKey++
    toast(`Filled in ${t.name} defaults. Check the address and the upstream (container name:port).`)
  }

  // Strip the editor-only defaults for handler types the site doesn't use.
  function payload() {
    const s = clone(site)
    const c = s.config
    const sh = c.access.shields
    for (const k of Object.keys(sh)) if (sh[k] === null || sh[k] === undefined || sh[k] === '__default') delete sh[k]
    sh.app_paths = (sh.app_paths || []).map((x) => x.trim()).filter(Boolean)
    sh.sso_users = (sh.sso_users || []).map((x) => x.trim()).filter(Boolean)
    c.access.crowdsec = false
    s.tags = tagText.split(',').map((t) => t.trim()).filter(Boolean)
    s.hosts = s.hosts.map((h) => h.trim()).filter(Boolean)
    if (c.forward_proxy) {
      c.forward_proxy.ports = portsText.split(',').map((x) => parseInt(x)).filter((x) => x > 0)
      c.forward_proxy.acl = (c.forward_proxy.acl || []).map((x) => x.trim()).filter(Boolean)
      c.forward_proxy.basic_auth = (c.forward_proxy.basic_auth || []).filter((u) => u.username?.trim() || u.password)
    }
    for (const k of ['proxy', 'static', 'redirect', 'respond', 'forward_proxy']) if (s.kind !== k) delete c[k]
    if (c.redirect) c.redirect.code = Number(c.redirect.code) || 302
    if (c.respond) c.respond.status = Number(c.respond.status) || 200
    c.access_log = accessLog === 'default' ? null : accessLog === 'on'
    c.access.allow_cidrs = c.access.allow_cidrs.map((x) => x.trim()).filter(Boolean)
    c.access.deny_cidrs = c.access.deny_cidrs.map((x) => x.trim()).filter(Boolean)
    c.headers_remove = c.headers_remove.map((x) => x.trim()).filter(Boolean)
    if (c.access.rate_limit && !c.access.rate_limit.events) delete c.access.rate_limit
    if (c.access.rate_limit) c.access.rate_limit.events = Number(c.access.rate_limit.events)
    if (c.access.forward_auth && !c.access.forward_auth.upstream) delete c.access.forward_auth
    for (const r of c.routes) for (const k of ['proxy', 'static', 'redirect', 'respond']) if (r.kind !== k) delete r[k]
    return s
  }

  async function save(review = false) {
    error = ''
    busy = true
    try {
      const body = payload()
      const saved = isNew ? await api.post('/sites', body) : await api.put('/sites/' + id, body)
      toast('Saved. Apply to make it live.')
      refreshStatus()
      if (review) go('/changes')
      else if (isNew) go('/sites/' + saved.id)
      else { site = ensure(saved); }
    } catch (e) { error = e.message }
    busy = false
  }

  async function remove() {
    if (!confirm(`Delete ${site.name}?`)) return
    try { await api.del('/sites/' + id); toast('Deleted'); refreshStatus(); go('/sites') } catch (e) { toastErr(e) }
  }
  async function duplicate() {
    try { const c = await api.post(`/sites/${id}/clone`); toast('Copy created (disabled)'); go('/sites/' + c.id) } catch (e) { toastErr(e) }
  }
  async function checkRaw() {
    rawCheck = null
    try { rawCheck = await api.post('/config/validate', { text: site.config.raw || '', context: 'site' }) } catch (e) { rawCheck = { valid: false, error: e.message } }
  }

  function addRoute() { site.config.routes.push({ path: '/api/*', kind: 'proxy', strip_prefix: false, proxy: { upstreams: [''] } }) }
  function routeKind(r, k) {
    r.kind = k
    if (k === 'proxy') r.proxy ||= { upstreams: [''] }
    if (k === 'static') r.static ||= { root: '' }
    if (k === 'redirect') r.redirect ||= { to: '', code: 302 }
    if (k === 'respond') r.respond ||= { status: 200, body: '' }
  }
  function toggleComp(alg, on) {
    const c = site.config
    c.compression = on ? [...new Set([...c.compression, alg])] : c.compression.filter((x) => x !== alg)
  }
  function toggleSnippet(name, on) {
    const c = site.config
    c.snippets = on ? [...new Set([...c.snippets, name])] : c.snippets.filter((x) => x !== name)
  }
  function ensureRL() { site.config.access.rate_limit ||= { events: 100, window: '1m', key: '' } }
  function ensureFA() { site.config.access.forward_auth ||= { preset: 'authelia', upstream: '', uri: '/api/authz/forward-auth', copy_headers: ['Remote-User', 'Remote-Groups', 'Remote-Email', 'Remote-Name'] } }
  function faPreset(p) {
    const fa = site.config.access.forward_auth
    fa.preset = p
    if (p === 'authelia') { fa.uri = '/api/authz/forward-auth'; fa.copy_headers = ['Remote-User', 'Remote-Groups', 'Remote-Email', 'Remote-Name'] }
    if (p === 'authentik') { fa.uri = '/outpost.goauthentik.io/auth/caddy'; fa.copy_headers = ['X-Authentik-Username', 'X-Authentik-Groups', 'X-Authentik-Email', 'X-Authentik-Name', 'X-Authentik-Uid'] }
  }
  const ro = $derived(!can.edit())
</script>

{#snippet sec(Icon, title, desc)}
  <span class="s-icon"><Icon size={16} /></span>
  <span class="s-text"><span>{title}</span><small>{desc}</small></span>
  <ChevronDown size={18} class="chev" />
{/snippet}

{#if !site}
  <div class="card skel-editor"></div>
{:else}
  <div class="head">
    <a class="btn ghost sm back" href="#/sites"><ArrowLeft size={16} /> Sites</a>
    <div class="head-main">
      <Avatar name={site.name || site.hosts.find((h) => h.trim()) || '?'} size={48} />
      <div class="grow">
        <h1 class="truncate">{isNew ? 'New site' : site.name}</h1>
        <div class="muted small truncate">{site.hosts.filter((h) => h.trim()).join(', ') || 'Add a hostname below'}</div>
      </div>
      {#if !isNew && can.edit()}
        <button class="btn sm" onclick={duplicate}><CopyPlus size={15} /> Duplicate</button>
        <button class="btn sm danger" onclick={remove}><Trash2 size={15} /> Delete</button>
      {/if}
    </div>
  </div>

  <fieldset class="stack" disabled={ro}>
    {#if isNew}
      <div class="card tpl">
        <span class="tpl-ic"><Sparkles size={18} /></span>
        <div class="grow"><strong>Start from a template</strong><div class="muted small">Sensible defaults for common self-hosted apps.</div></div>
        <select style="max-width:240px" onchange={(e) => applyTemplate(templates.find((t) => t.id === e.target.value))}>
          <option value="">Choose an app…</option>
          {#each templates as t}<option value={t.id}>{t.name}</option>{/each}
        </select>
      </div>
    {/if}

    <div class="card stack">
      <div class="card-head"><div><h2>Basics</h2><div class="desc">Where this site lives.</div></div>
        <label class="row tight right enabled"><Switch bind:checked={site.enabled} label="Enabled" /> <span class="small">{site.enabled ? 'Enabled' : 'Disabled'}</span></label>
      </div>
      <div class="field">
        <span>Hostnames</span>
        {#key hostKey}<HostInput bind:hosts={site.hosts} domains={domainList} {defaultDomain} />{/key}
        <span class="hint">The address people type in their browser. HTTPS certificates are obtained automatically.</span>
      </div>
      <div class="grid2">
        <label class="field"><span>Display name</span><input type="text" bind:value={site.name} placeholder="Defaults to the hostname" /></label>
        <label class="field"><span>Tags</span><input type="text" bind:value={tagText} placeholder="media, home" /></label>
      </div>
    </div>

    <div class="card stack">
      <div class="card-head"><div><h2>What should this site do?</h2><div class="desc">Pick how Caddy handles requests.</div></div></div>
      <div class="kinds">
        {#each [['proxy', Server, 'Reverse proxy', 'Forward to an app'], ['static', Folder, 'File server', 'Serve a folder'], ['redirect', CornerUpRight, 'Redirect', 'Send elsewhere'], ['respond', MessageSquare, 'Response', 'Fixed text & status'], ['forward_proxy', ArrowLeftRight, 'Forward proxy', 'HTTPS proxy for devices'], ['none', Code, 'Custom', 'Raw Caddyfile']] as [k, Icon, label, desc]}
          <label class="kind" class:active={site.kind === k}>
            <input type="radio" name="kind" value={k} bind:group={site.kind} />
            <span class="k-ic"><Icon size={18} /></span>
            <strong>{label}</strong><span class="muted xs">{desc}</span>
          </label>
        {/each}
      </div>

      {#if site.kind === 'proxy'}
        <div class="field">
          <span class="row tight">Upstream address <button class="btn xs right" type="button" onclick={() => (scanning = true)}><Radar size={14} /> Find app</button></span>
          <ListInput bind:items={site.config.proxy.upstreams} placeholder="jellyfin:8096" addLabel="Add upstream" />
          <span class="hint">Where the app listens, as <code>address:port</code>. For a Docker container on a network shared with Caddy, use its container name (<code>jellyfin:8096</code>). For anything else, use the machine's IP (<code>10.0.1.5:32400</code>). Add more than one to load-balance.</span>
        </div>
        <div class="checks">
          <label class="check"><input type="checkbox" bind:checked={site.config.proxy.websocket} /> WebSockets / streaming</label>
          <label class="check"><input type="checkbox" bind:checked={site.config.proxy.tls_skip_verify} /> Upstream uses a self-signed certificate</label>
          <label class="check"><input type="checkbox" bind:checked={site.config.proxy.h2c} /> gRPC / h2c upstream</label>
        </div>
      {:else if site.kind === 'static'}
        <label class="field"><span>Folder</span><input type="text" bind:value={site.config.static.root} placeholder="/srv/www" />
          <span class="hint">Path inside the Caddy container.</span></label>
        <div class="checks">
          <label class="check"><input type="checkbox" bind:checked={site.config.static.browse} /> Show directory listings</label>
          <label class="check"><input type="checkbox" bind:checked={site.config.static.spa} /> Single-page app (fall back to index.html)</label>
        </div>
      {:else if site.kind === 'redirect'}
        <div class="grid2">
          <label class="field"><span>Redirect to</span><input type="text" bind:value={site.config.redirect.to} placeholder="https://example.com" /></label>
          <label class="field"><span>Type</span>
            <select bind:value={site.config.redirect.code}>
              <option value={302}>Temporary (302)</option><option value={301}>Permanent (301)</option>
              <option value={307}>Temporary, keep method (307)</option><option value={308}>Permanent, keep method (308)</option>
            </select></label>
        </div>
        <label class="check"><input type="checkbox" bind:checked={site.config.redirect.keep_path} /> Keep the path (/foo → target/foo)</label>
      {:else if site.kind === 'respond'}
        <div class="grid2">
          <label class="field"><span>Status code</span><input type="number" bind:value={site.config.respond.status} /></label>
          <label class="field"><span>Content type</span><input type="text" bind:value={site.config.respond.content_type} placeholder="text/plain (optional)" /></label>
        </div>
        <label class="field"><span>Body</span><textarea rows="4" bind:value={site.config.respond.body}></textarea></label>
      {:else if site.kind === 'forward_proxy'}
        {#if !available('forward_proxy')}
          <div class="alert warn"><TriangleAlert /><span>The <code>forwardproxy</code> plugin isn't in your Caddy build. <a href="#/plugins">Install it under Plugins</a>, then apply.</span></div>
        {/if}
        <p class="muted small">Point devices at <span class="mono">https://{site.hosts.find((h) => h.trim()) || 'proxy.example.com'}:443</span> as their HTTPS proxy. Logins are kept as plain text because the plugin compares them as-is; use a dedicated password.</p>
        <div class="field">
          <span>Logins</span>
          {#each site.config.forward_proxy.basic_auth as u, i}
            <div class="row">
              <input type="text" bind:value={u.username} placeholder="username" style="max-width:200px" autocomplete="off" />
              <input type="password" bind:value={u.password} placeholder="password" style="max-width:220px" autocomplete="new-password" />
              <button class="btn ghost icon sm" type="button" aria-label="Remove login" onclick={() => site.config.forward_proxy.basic_auth.splice(i, 1)}><Trash2 size={15} /></button>
            </div>
          {/each}
          <div><button class="btn xs" type="button" onclick={() => site.config.forward_proxy.basic_auth.push({ username: '', password: '' })}><Plus size={14} /> Add login</button></div>
          <span class="hint">Without a login the proxy is open to anyone who finds it. Combine with the IP rules under Access control.</span>
        </div>
        <div class="checks">
          <label class="check"><input type="checkbox" bind:checked={site.config.forward_proxy.hide_ip} /> Hide client IPs (no X-Forwarded-For)</label>
          <label class="check"><input type="checkbox" bind:checked={site.config.forward_proxy.hide_via} /> Hide the Via header</label>
        </div>
        <div class="grid2">
          <label class="field"><span>Allowed destination ports</span><input type="text" bind:value={portsText} placeholder="80, 443 (empty = any)" /></label>
          <label class="field"><span>Probe resistance secret</span><input type="text" bind:value={site.config.forward_proxy.probe_resistance} placeholder="secret.localhost (optional)" />
            <span class="hint">Only requests for this hostname reveal the proxy; to everyone else it looks like a normal site.</span></label>
          <label class="field"><span>Dial timeout</span><input type="text" bind:value={site.config.forward_proxy.dial_timeout} placeholder="30s" /></label>
          <label class="field"><span>Serve a PAC file at</span><input type="text" bind:value={site.config.forward_proxy.serve_pac} placeholder="/proxy.pac (optional)" /></label>
          <label class="field"><span>Chain to another proxy</span><input type="text" bind:value={site.config.forward_proxy.upstream} placeholder="https://user:pass@host:port (optional)" /></label>
        </div>
        <div class="field"><span>Access rules (checked in order)</span>
          <ListInput bind:items={site.config.forward_proxy.acl} placeholder="deny 10.0.0.0/8" addLabel="Add rule" />
          <span class="hint"><code>allow</code> or <code>deny</code> followed by a hostname, IP, range or <code>all</code>, e.g. <code>deny 192.168.0.0/16</code> then <code>allow all</code>.</span>
        </div>
      {:else}
        <div class="alert info"><Info /><span>Write your directives under <strong>Custom Caddyfile</strong> below. They go inside this site's block.</span></div>
      {/if}
    </div>

    {#if site.kind === 'proxy'}
      <details class="section">
        <summary>{@render sec(Activity, 'Load balancing & health checks', 'Policies, retries, timeouts and upstream headers')}</summary>
        <div class="body">
          <div class="grid2">
            <label class="field"><span>Load balancing policy</span>
              <select bind:value={site.config.proxy.lb_policy}>
                <option value="">Default (random)</option>
                {#each ['round_robin', 'least_conn', 'first', 'ip_hash', 'client_ip_hash', 'uri_hash', 'random_choose 2'] as p}<option value={p}>{p}</option>{/each}
              </select></label>
            <label class="field"><span>Retry for</span><input type="text" bind:value={site.config.proxy.lb_try_duration} placeholder="e.g. 5s" /></label>
            <label class="field"><span>Health check path</span><input type="text" bind:value={site.config.proxy.health_uri} placeholder="/health" /></label>
            <label class="field"><span>Check interval</span><input type="text" bind:value={site.config.proxy.health_interval} placeholder="30s" /></label>
            <label class="field"><span>Mark down after fails</span><input type="number" bind:value={site.config.proxy.max_fails} placeholder="1" /></label>
            <label class="field"><span>Remember fails for</span><input type="text" bind:value={site.config.proxy.fail_duration} placeholder="30s" /></label>
            <label class="field"><span>Dial timeout</span><input type="text" bind:value={site.config.proxy.dial_timeout} placeholder="e.g. 5s" /></label>
            <label class="field"><span>Response header timeout</span><input type="text" bind:value={site.config.proxy.response_header_timeout} placeholder="e.g. 30s" /></label>
            <label class="field"><span>Upstream TLS server name</span><input type="text" bind:value={site.config.proxy.tls_server_name} placeholder="(optional)" /></label>
            <label class="field"><span>Flush interval</span><input type="text" bind:value={site.config.proxy.flush_interval} placeholder="-1 for streaming" /></label>
          </div>
          <div class="field"><span>Headers sent to the upstream</span><KVInput bind:value={site.config.proxy.header_up} keyPlaceholder="Header" valuePlaceholder="Value (or -Header to remove)" addLabel="Add header" /></div>
          <div class="field"><span>Headers changed in the response</span><KVInput bind:value={site.config.proxy.header_down} keyPlaceholder="Header" addLabel="Add header" /></div>
        </div>
      </details>
    {/if}

    <details class="section">
      <summary>{@render sec(Route, 'Path routes', site.config.routes.length ? `${site.config.routes.length} route(s)` : 'Send specific paths somewhere else')}</summary>
      <div class="body">
        <p class="muted small">For example <code>/api/*</code> to a backend. Checked in order, before the main handler.</p>
        {#each site.config.routes as r, i}
          <div class="route stack tight">
            <div class="row">
              <input type="text" bind:value={r.path} placeholder="/api/*" style="max-width:200px" class="mono" />
              <select value={r.kind} onchange={(e) => routeKind(r, e.target.value)} style="max-width:170px">
                <option value="proxy">Reverse proxy</option><option value="static">File server</option>
                <option value="redirect">Redirect</option><option value="respond">Response</option>
              </select>
              <label class="check small"><input type="checkbox" bind:checked={r.strip_prefix} /> Strip prefix</label>
              <button class="btn ghost icon sm right" type="button" aria-label="Remove route" onclick={() => site.config.routes.splice(i, 1)}><Trash2 size={15} /></button>
            </div>
            {#if r.kind === 'proxy'}<ListInput bind:items={r.proxy.upstreams} placeholder="localhost:9000" addLabel="Add upstream" />
            {:else if r.kind === 'static'}<input type="text" bind:value={r.static.root} placeholder="/srv/files" />
            {:else if r.kind === 'redirect'}<input type="text" bind:value={r.redirect.to} placeholder="https://…" />
            {:else}<div class="row"><input type="number" bind:value={r.respond.status} style="max-width:100px" /><input type="text" bind:value={r.respond.body} placeholder="Body" class="grow" /></div>{/if}
          </div>
        {/each}
        <div><button class="btn sm" type="button" onclick={addRoute}><Plus size={15} /> Add route</button></div>
      </div>
    </details>

    <details class="section">
      <summary>{@render sec(ShieldCheck, 'HTTPS & redirects', 'Certificate source, HTTP→HTTPS and www handling')}</summary>
      <div class="body">
        <div class="grid2">
          <label class="field"><span>Certificate</span>
            <select bind:value={site.config.tls.mode}>
              <option value="auto">Automatic (Let's Encrypt / domain settings)</option>
              <option value="internal">Caddy internal CA (LAN / self-signed)</option>
              <option value="custom">My own certificate files</option>
              <option value="off">Off (plain HTTP only)</option>
            </select></label>
          <label class="field"><span>www handling</span>
            <select bind:value={site.config.www_redirect}>
              <option value="">Leave as is</option><option value="to_apex">Redirect www → bare domain</option><option value="to_www">Redirect bare domain → www</option>
            </select></label>
        </div>
        {#if site.config.tls.mode === 'custom'}
          <div class="grid2">
            <label class="field"><span>Certificate file</span><input type="text" bind:value={site.config.tls.cert_file} placeholder="/certs/site.crt" /></label>
            <label class="field"><span>Key file</span><input type="text" bind:value={site.config.tls.key_file} placeholder="/certs/site.key" /></label>
          </div>
        {/if}
        {#if site.config.tls.mode !== 'off'}
          <label class="check"><input type="checkbox" checked={site.config.http_redirect !== false} onchange={(e) => (site.config.http_redirect = e.target.checked ? null : false)} /> Redirect HTTP to HTTPS</label>
        {/if}
      </div>
    </details>

    <details class="section">
      <summary>{@render sec(Lock, 'Access control', 'Protection, login wall, IP rules and rate limits')}</summary>
      <div class="body">

        <div class="field">
          <span>Protection <a class="small" href="#/security" style="font-weight:400">(defaults on the Security page)</a></span>
          <div class="shields">
            {#each shieldRows as [k, label]}
              <label class="shield-row"><span class="grow">{label}</span>
                <select value={triValue(k)} onchange={(e) => setTri(k, e.target.value)}>
                  <option value="__default">Default{secDefaults ? ` (${onOff(secDefaults[k])})` : ''}</option>
                  <option value="on">On</option>
                  <option value="off">Off</option>
                </select></label>
            {/each}
            <label class="shield-row"><span class="grow">Login wall</span>
              <select value={sh.login ?? '__default'} onchange={(e) => (sh.login = e.target.value === '__default' ? null : e.target.value)}>
                <option value="__default">Default{secDefaults ? ` (${loginNames[secDefaults.login || '']})` : ''}</option>
                <option value="">No login wall</option>
                <option value="sso">Sign-in with two-factor</option>
                <option value="basic">Shared password</option>
              </select></label>
          </div>
        </div>

        {#if login}
          <div class="grid2">
            <label class="field"><span>Phone and TV apps</span>
              <select bind:value={sh.apps}>
                <option value="">None: the login wall covers everything</option>
                {#each appPresets as [pid, pname]}<option value={pid}>{pname}</option>{/each}
                <option value="custom">Custom paths…</option>
              </select>
              <span class="hint">Apps can't get past a login wall. Picking the app leaves the paths it uses to the app's own sign-in, while the browser pages stay behind the wall.</span></label>
            {#if login === 'sso'}
              <div class="field"><span>Only these accounts</span><ListInput bind:items={sh.sso_users} placeholder="username" addLabel="Add account" />
                <span class="hint">Empty: every Caddymium account can sign in to this site.</span></div>
            {/if}
          </div>
          {#if sh.apps === 'custom'}
            <div class="grid2">
              <label class="field"><span>The login wall covers</span>
                <select bind:value={sh.app_mode}><option value="except">Everything except these paths</option><option value="only">Only these paths</option></select></label>
              <div class="field"><span>Paths</span><ListInput bind:items={sh.app_paths} placeholder="/api/*" addLabel="Add path" /></div>
            </div>
          {/if}
        {/if}
        <div class="field">
          <span>Extra passwords for this site only</span>
          {#each site.config.access.basic_auth as u, i}
            <div class="row">
              <input type="text" bind:value={u.username} placeholder="username" style="max-width:200px" />
              {#if u.hash?.startsWith('$2')}
                <span class="badge ok"><Check /> password set</span>
                <button class="btn xs" type="button" onclick={() => (u.hash = '')}>Change</button>
              {:else}
                <input type="password" bind:value={u.hash} placeholder="new password" style="max-width:220px" autocomplete="new-password" />
              {/if}
              <button class="btn ghost icon sm" type="button" aria-label="Remove user" onclick={() => site.config.access.basic_auth.splice(i, 1)}><Trash2 size={15} /></button>
            </div>
          {/each}
          <div><button class="btn xs" type="button" onclick={() => site.config.access.basic_auth.push({ username: '', hash: '' })}><Plus size={14} /> Add user</button></div>
        </div>
        <div class="grid2">
          <div class="field"><span>Only allow these IPs / ranges</span><ListInput bind:items={site.config.access.allow_cidrs} placeholder="192.168.1.0/24" addLabel="Add range" />
            <span class="hint">Empty = everyone. Use <code>private_ranges</code> for LAN only.</span></div>
          <div class="field"><span>Block these IPs / ranges</span><ListInput bind:items={site.config.access.deny_cidrs} placeholder="203.0.113.7" addLabel="Add range" /></div>
        </div>
        <div class="field">
          <span>Single sign-on (forward auth)</span>
          {#if site.config.access.forward_auth}
            <div class="grid2">
              <label class="field"><span>Provider</span>
                <select value={site.config.access.forward_auth.preset} onchange={(e) => faPreset(e.target.value)}>
                  <option value="authelia">Authelia</option><option value="authentik">Authentik</option><option value="custom">Other</option>
                </select></label>
              <label class="field"><span>Auth server address</span><input type="text" bind:value={site.config.access.forward_auth.upstream} placeholder="authelia:9091" /></label>
              <label class="field"><span>Verify URI</span><input type="text" bind:value={site.config.access.forward_auth.uri} /></label>
            </div>
            <div><button class="btn ghost xs" type="button" onclick={() => delete site.config.access.forward_auth}>Remove SSO</button></div>
          {:else}
            <div><button class="btn xs" type="button" onclick={ensureFA}><Fingerprint size={14} /> Protect with Authelia / Authentik</button></div>
          {/if}
        </div>
        <div class="field">
          <span>Rate limit {#if !available('rate_limit')}<span class="badge warn">plugin not installed</span>{/if}</span>
          {#if site.config.access.rate_limit}
            <div class="row">
              <input type="number" bind:value={site.config.access.rate_limit.events} style="max-width:110px" /> <span class="muted">requests per</span>
              <input type="text" bind:value={site.config.access.rate_limit.window} style="max-width:90px" /> <span class="muted">per client</span>
              <button class="btn ghost xs" type="button" onclick={() => delete site.config.access.rate_limit}>Remove</button>
            </div>
          {:else}
            <div><button class="btn xs" type="button" onclick={ensureRL} disabled={!available('rate_limit')}><Plus size={14} /> Add rate limit</button></div>
          {/if}
        </div>
      </div>
    </details>

    <details class="section">
      <summary>{@render sec(Gauge, 'Headers, compression & limits', 'Response headers, gzip/zstd, upload size, logging')}</summary>
      <div class="body">
        <div class="checks row">
          <label class="check"><input type="checkbox" checked={site.config.compression.includes('zstd')} onchange={(e) => toggleComp('zstd', e.target.checked)} /> zstd compression</label>
          <label class="check"><input type="checkbox" checked={site.config.compression.includes('gzip')} onchange={(e) => toggleComp('gzip', e.target.checked)} /> gzip compression</label>
        </div>
        <div class="field"><span>Set response headers</span><KVInput bind:value={site.config.headers_set} keyPlaceholder="Header" addLabel="Add header" /></div>
        <div class="field"><span>Remove response headers</span><ListInput bind:items={site.config.headers_remove} placeholder="Server" addLabel="Add header" /></div>
        <div class="grid2">
          <label class="field"><span>Max upload size</span><input type="text" bind:value={site.config.max_body_size} placeholder="e.g. 100MB (empty = unlimited)" /></label>
          <label class="field"><span>Access log</span>
            <select bind:value={accessLog}><option value="default">Default (from settings)</option><option value="on">On</option><option value="off">Off</option></select></label>
        </div>
      </div>
    </details>

    <details class="section">
      <summary>{@render sec(Construction, 'Maintenance mode', site.config.maintenance ? 'On: visitors see a maintenance page' : 'Temporarily show a “be right back” page')}</summary>
      <div class="body">
        <label class="check"><input type="checkbox" bind:checked={site.config.maintenance} /> Show a maintenance page instead of the site (HTTP 503)</label>
        {#if site.config.maintenance}
          <label class="field"><span>Message</span><textarea rows="3" bind:value={site.config.maintenance_body} placeholder="We'll be right back."></textarea></label>
        {/if}
      </div>
    </details>

    <details class="section" open={site.kind === 'none' || !!site.config.raw}>
      <summary>{@render sec(Code, 'Custom Caddyfile', site.config.raw ? 'In use' : 'Snippets and extra directives for anything not covered above')}</summary>
      <div class="body">
        {#if snippets.length}
          <div class="field"><span>Include snippets</span>
            <div class="checks">
              {#each snippets as sn}
                <label class="check"><input type="checkbox" checked={site.config.snippets.includes(sn.name)} onchange={(e) => toggleSnippet(sn.name, e.target.checked)} /> <span class="mono">{sn.name}</span> <span class="muted small">{sn.description}</span></label>
              {/each}
            </div>
          </div>
        {/if}
        <label class="field"><span>Extra directives (inside this site's block)</span>
          <textarea rows="8" bind:value={site.config.raw} placeholder={'# anything Caddy supports, e.g.\nheader /static/* Cache-Control "max-age=86400"'}></textarea>
        </label>
        <div class="row">
          <button class="btn sm" type="button" onclick={checkRaw}><Check size={15} /> Check syntax</button>
          {#if rawCheck}{#if rawCheck.valid}<span class="badge ok"><Check /> looks valid</span>{:else}<span class="mono small err">{rawCheck.error}</span>{/if}{/if}
        </div>
      </div>
    </details>
  </fieldset>

  {#if can.edit()}
    <div class="actionbar">
      {#if error}<ErrorBox class="grow" message={error} />{:else}<span class="muted small grow hide-sm">Saved changes go live after you review and apply.</span>{/if}
      <button class="btn" onclick={() => save(false)} disabled={busy}>Save</button>
      <button class="btn primary" onclick={() => save(true)} disabled={busy}>Save & review <ArrowRight size={15} /></button>
    </div>
  {/if}
{/if}

{#if scanning}<ScanModal onpick={usePick} onclose={() => (scanning = false)} />{/if}

<style>
  .shields { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 6px 14px; }
  .shield-row { display: flex; align-items: center; gap: 10px; font-size: 13.5px; padding: 4px 0; }
  .shield-row select { max-width: 150px; padding: 4px 8px; font-size: 13px; }
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  .head { display: flex; flex-direction: column; gap: 14px; margin-bottom: 22px; }
  .back { align-self: flex-start; margin-left: -8px; }
  .head-main { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
  .head-main h1 { font-size: 22px; }
  .tpl { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; padding: 14px 16px;
    background: linear-gradient(120deg, var(--accent-soft), var(--surface) 70%); }
  .tpl-ic { width: 36px; height: 36px; border-radius: 10px; display: grid; place-items: center; background: var(--surface); color: var(--accent); border: 1px solid var(--border); flex: none; }
  .enabled { cursor: pointer; }
  .kinds { display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 8px; }
  .kind {
    position: relative; display: flex; flex-direction: column; gap: 2px; padding: 12px; border: 1px solid var(--border);
    border-radius: 10px; cursor: pointer; background: var(--surface); transition: border-color 0.15s, background 0.15s, box-shadow 0.15s;
  }
  .kind:hover { border-color: var(--border-strong); }
  .kind input { position: absolute; opacity: 0; pointer-events: none; }
  .kind:has(input:focus-visible) { box-shadow: 0 0 0 3px var(--ring); }
  .k-ic { width: 30px; height: 30px; border-radius: 8px; display: grid; place-items: center; background: var(--surface-2); color: var(--muted); margin-bottom: 6px; transition: 0.15s; }
  .kind.active { border-color: var(--accent); background: var(--accent-soft); box-shadow: 0 0 0 1px var(--accent) inset; }
  .kind.active .k-ic { background: var(--accent); color: var(--accent-text); }
  .checks { display: flex; flex-direction: column; gap: 10px; }
  .checks.row { flex-direction: row; gap: 20px; }
  .route { border: 1px solid var(--border); background: var(--surface-2); border-radius: 10px; padding: 12px; }
  .actionbar {
    position: sticky; bottom: 16px; z-index: 5; display: flex; gap: 8px; align-items: center; margin-top: 18px;
    padding: 10px 10px 10px 16px; background: color-mix(in srgb, var(--surface) 88%, transparent); backdrop-filter: blur(10px);
    border: 1px solid var(--border); border-radius: 14px; box-shadow: var(--shadow-lg);
  }
  .actionbar .alert { padding: 7px 10px; }
  .err { color: var(--danger); }
  .skel-editor { height: 420px; }
</style>
