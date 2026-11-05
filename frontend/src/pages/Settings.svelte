<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { onMount } from 'svelte'
  import { api } from '../lib/api.js'
  import { app, toast, toastErr, refreshStatus, loadMeta, can } from '../lib/state.svelte.js'
  import ListInput from '../lib/ListInput.svelte'
  import { knownDomains } from '../lib/domains.js'
  import { download } from '../lib/util.js'

  let restoring = $state(false)
  let restored = $state(null)
  async function backup() {
    try {
      const env = await api.get('/backup')
      const stamp = new Date().toISOString().slice(0, 16).replace(/[-:T]/g, '')
      download(`caddymium-backup-${stamp}.json`, JSON.stringify(env, null, 2))
      toast('Backup downloaded. It is encrypted with your CADDYMIUM_SECRET_KEY; keep the key with it.')
    } catch (e) { toastErr(e) }
  }
  async function restoreFile(ev) {
    const file = ev.target.files?.[0]
    ev.target.value = ''
    if (!file) return
    let env
    try { env = JSON.parse(await file.text()) } catch { toast('That file is not a Caddymium backup.', 'err'); return }
    const made = env.created_at ? new Date(env.created_at).toLocaleString() : 'unknown date'
    const n = Object.entries(env.contents || {}).map(([k, v]) => `${v} ${k.replace('_', ' ')}`).join(', ')
    if (!confirm(`Restore the backup from ${made}?\n\nIt holds: ${n || 'unknown contents'}.\n\nThis REPLACES all sites, domains, DNS accounts, snippets, raw blocks, templates, notification channels and settings. Users and API keys are kept. Nothing is applied to Caddy until you review and apply.`)) return
    restoring = true
    restored = null
    try {
      restored = await api.post('/backup/restore', env)
      s = await api.get('/settings')
      toast('Backup restored. Review & apply to put it into Caddy.')
      refreshStatus()
    } catch (e) { toastErr(e) }
    restoring = false
  }

  let s = $state(null)
  let tab = $state('certs')
  let eab = $state('')
  let csKey = $state('')
  let error = $state('')
  let busy = $state(false)
  let alertDays = $state('')
  let domainList = $state([])

  onMount(async () => {
    loadMeta().catch(() => {})
    try {
      s = await api.get('/settings')
      s.resolvers ||= []
      s.scan_ranges ||= []
      s.trusted_proxies ||= { enabled: false, presets: [], custom: [] }
      s.trusted_proxies.presets ||= []
      s.trusted_proxies.custom ||= []
      alertDays = (s.cert_alert_days || []).join(', ')
      const [doms, sites] = await Promise.all([api.get('/domains').catch(() => []), api.get('/sites').catch(() => [])])
      domainList = knownDomains(doms, sites, s.default_domain)
    } catch (e) { toastErr(e) }
  })

  async function save() {
    error = ''
    busy = true
    try {
      const body = { ...s, cert_alert_days: alertDays.split(',').map((x) => parseInt(x)).filter((x) => x > 0) }
      body.resolvers = (body.resolvers || []).map((x) => x.trim()).filter(Boolean)
      body.scan_ranges = (body.scan_ranges || []).map((x) => x.trim()).filter(Boolean)
      body.trusted_proxies.custom = body.trusted_proxies.custom.map((x) => x.trim()).filter(Boolean)
      if (eab) body.acme_eab_mac = eab
      if (csKey) body.crowdsec_api_key = csKey
      s = await api.put('/settings', body)
      s.trusted_proxies.custom ||= []
      s.trusted_proxies.presets ||= []
      s.scan_ranges ||= []
      eab = ''; csKey = ''
      toast('Settings saved. Apply to update Caddy.')
      refreshStatus()
    } catch (e) { error = e.message }
    busy = false
  }
  function togglePreset(p, on) {
    const tp = s.trusted_proxies
    tp.presets = on ? [...new Set([...tp.presets, p])] : tp.presets.filter((x) => x !== p)
  }
  const ro = $derived(!can.admin())
</script>

<div class="page-head">
  <div>
    <h1>Settings</h1>
    <div class="sub">Global options for the whole Caddy server.</div>
  </div>
</div>

{#if s}
  <div class="tabs">
    {#each [['certs', 'Domains & certificates'], ['network', 'Proxies & DNS'], ['logs', 'Logs & metrics'], ['advanced', 'Advanced']] as [k, l]}
      <button class:active={tab === k} onclick={() => (tab = k)}>{l}</button>
    {/each}
  </div>

  <fieldset class="stack" disabled={ro}>
    {#if tab === 'certs'}
      <div class="card stack">
        <label class="field"><span>Default domain for new sites</span>
          {#if domainList.length}
            <select bind:value={s.default_domain}>
              <option value="">First one found</option>
              {#each domainList as d}<option value={d}>{d}</option>{/each}
            </select>
          {:else}
            <input type="text" bind:value={s.default_domain} placeholder="example.com" />
          {/if}
          <span class="hint">New sites start as <code>name.{s.default_domain || domainList[0] || 'example.com'}</code>; you only type the name. The list comes from Domains &amp; DNS and the domains your sites already use.</span>
        </label>
      </div>
      <div class="card stack">
        <div class="grid2">
          <label class="field"><span>Email for certificate notices</span><input type="email" bind:value={s.acme_email} placeholder="you@example.com" /></label>
          <label class="field"><span>Certificate authority</span>
            <select bind:value={s.acme_ca}>
              <option value="letsencrypt">Let's Encrypt (default)</option>
              <option value="letsencrypt_staging">Let's Encrypt staging (testing)</option>
              <option value="zerossl">ZeroSSL</option>
              <option value="google">Google Trust Services</option>
              <option value="google_staging">Google staging (testing)</option>
              <option value="custom">Custom ACME server</option>
            </select></label>
        </div>
        {#if s.acme_ca === 'custom'}
          <div class="grid2">
            <label class="field"><span>ACME directory URL</span><input type="url" bind:value={s.acme_ca_url} placeholder="https://ca.internal/acme/directory" /></label>
            <label class="field"><span>CA root certificate (PEM path)</span><input type="text" bind:value={s.acme_ca_root} placeholder="/certs/root.pem (optional)" /></label>
          </div>
        {/if}
        {#if s.acme_ca === 'zerossl' || s.acme_ca === 'google' || s.acme_ca === 'custom'}
          <div class="grid2">
            <label class="field"><span>EAB key ID</span><input type="text" bind:value={s.acme_eab_key_id} placeholder="(if your CA requires it)" /></label>
            <label class="field"><span>EAB HMAC key</span><input type="password" bind:value={eab} placeholder={s.acme_eab_mac_set ? '•••••• (saved)' : ''} autocomplete="off" /></label>
          </div>
        {/if}
        <label class="field"><span>Warn me when a certificate expires within (days)</span><input type="text" bind:value={alertDays} placeholder="14, 7" /></label>
      </div>
    {:else if tab === 'network'}
      <div class="card stack">
        <h2>DNS propagation check</h2>
        <p class="muted small">Used with DNS validation to confirm records are visible. Pick public resolvers if your network has local DNS overrides for your domains.</p>
        <label class="field"><span>Resolvers</span>
          <select bind:value={s.resolver_preset}>
            <option value="quad9">Quad9</option><option value="cloudflare">Cloudflare</option><option value="google">Google</option>
            <option value="system">System default</option><option value="custom">Custom</option>
          </select></label>
        {#if s.resolver_preset === 'custom'}<ListInput bind:items={s.resolvers} placeholder="9.9.9.9" addLabel="Add resolver" />{/if}
        <div class="grid2">
          <label class="field"><span>Propagation delay</span><input type="text" bind:value={s.propagation_delay} placeholder="e.g. 30s (optional)" /></label>
          <label class="field"><span>Propagation timeout</span><input type="text" bind:value={s.propagation_timeout} placeholder="e.g. 5m (optional)" /></label>
        </div>
      </div>
      <div class="card stack">
        <h2>Find app ranges</h2>
        <p class="muted small">Networks the “Find app” button scans, besides Docker. The network of the address you open Caddymium at (e.g. 10.0.1.5 → 10.0.1.0/24) is always included. Add other subnets or VLANs here.</p>
        <ListInput bind:items={s.scan_ranges} placeholder="10.0.2.0/24" addLabel="Add range" />
      </div>
      <div class="card stack">
        <h2>Behind a CDN or another proxy?</h2>
        <p class="muted small">Trust these proxies so logs and IP rules see the real visitor IP. Leave off unless traffic actually comes through them.</p>
        <label class="check"><input type="checkbox" bind:checked={s.trusted_proxies.enabled} /> Trust proxy headers</label>
        {#if s.trusted_proxies.enabled}
          <div class="row">
            {#each [['cloudflare', 'Cloudflare'], ['fastly', 'Fastly'], ['private', 'Private networks (LAN, Docker)']] as [p, l]}
              <label class="check"><input type="checkbox" checked={s.trusted_proxies.presets.includes(p)} onchange={(e) => togglePreset(p, e.target.checked)} /> {l}</label>
            {/each}
          </div>
          <div class="field"><span>Other ranges</span><ListInput bind:items={s.trusted_proxies.custom} placeholder="203.0.113.0/24" addLabel="Add range" /></div>
        {/if}
      </div>
    {:else if tab === 'logs'}
      <div class="card stack">
        <label class="check"><input type="checkbox" bind:checked={s.default_access_log} /> Write access logs for every site (can be changed per site)</label>
        <div class="grid2">
          <label class="field"><span>Rotate log files at</span><input type="text" bind:value={s.log_roll_size} placeholder="10MiB" /></label>
          <label class="field"><span>Keep rotated files</span><input type="number" bind:value={s.log_roll_keep} /></label>
        </div>
        <label class="check"><input type="checkbox" bind:checked={s.metrics} /> Enable Prometheus metrics (per site)</label>
      </div>
    {:else}
      <div class="card stack">
        <label class="check"><input type="checkbox" bind:checked={s.dry_run} /> Safety mode: every apply is a dry run (validate only)</label>
        <div class="grid2">
          <label class="field"><span>Log out after inactivity (minutes)</span><input type="number" bind:value={s.session_timeout_minutes} /></label>
          <label class="field"><span>Serve Caddymium itself at</span><input type="text" bind:value={s.panel_domain} placeholder="caddymium.example.com (optional)" />
            <span class="hint">Caddy will proxy this hostname to the panel with HTTPS.</span></label>
        </div>
        <p class="hint">CrowdSec, bot blocking, login walls and home-only access are on the <a href="#/security">Security</a> page.</p>
        <label class="field"><span>Extra global options</span>
          <textarea rows="8" bind:value={s.raw_global} placeholder="servers {'{'}&#10;&#9;timeouts {'{'}&#10;&#9;&#9;read_body 10s&#10;&#9;{'}'}&#10;{'}'}"></textarea>
          <span class="hint">Placed inside the global <code>{'{ }'}</code> block. Checked by Caddy before every apply.</span></label>
      </div>
      {#if can.admin()}
        <div class="card stack">
          <h2>Backup &amp; restore</h2>
          <p class="hint">A backup holds every site, domain, DNS account, snippet, raw block, template, notification channel, named secret and these settings, encrypted with the panel's <code>CADDYMIUM_SECRET_KEY</code>. Keep the key with the file: a restore needs the same key. Users, API keys and the history are not included; certificates live in Caddy's data volume.</p>
          <div class="row" style="gap:8px; flex-wrap:wrap">
            <button class="btn primary" type="button" onclick={backup}>Download backup</button>
            <label class="btn" style="cursor:pointer">Restore from file… <input type="file" accept=".json,application/json" style="display:none" onchange={restoreFile} disabled={restoring} /></label>
          </div>
          {#if restored}
            <div class="alert ok"><span>Restored a backup from {restored.created_at ? new Date(restored.created_at).toLocaleString() : 'an unknown date'}: {Object.entries(restored.restored?.tables || {}).map(([k, v]) => `${v} ${k.replace('_', ' ')}`).join(', ')}. Go to <a href="#/changes">Review &amp; apply</a> to put it into Caddy.</span></div>
          {/if}
        </div>
      {/if}
    {/if}
  </fieldset>

  {#if error}<ErrorBox message={error} />{/if}
  {#if can.admin()}
    <div class="row" style="margin-top:14px"><button class="btn primary" onclick={save} disabled={busy}>Save settings</button></div>
  {/if}
{:else}
  <p class="muted">Loading…</p>
{/if}

<style>
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
</style>
