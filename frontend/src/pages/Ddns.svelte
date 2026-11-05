<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { onMount } from 'svelte'
  import { Signal, Plus, Trash2, RefreshCw, CircleCheck, CircleX, TriangleAlert, Info } from '@lucide/svelte'
  import { api } from '../lib/api.js'
  import { app, toast, toastErr, refreshStatus, loadMeta, can } from '../lib/state.svelte.js'
  import { when } from '../lib/util.js'

  let s = $state(null)
  let creds = $state([])
  let status = $state(null)
  let checking = $state(false)
  let busy = $state(false)
  let error = $state('')

  function ensure(settings) {
    const d = settings.ddns || {}
    d.domains ||= []
    d.ip_sources ||= []
    d.check_interval ||= '5m'
    if (d.ipv4 == null) d.ipv4 = true
    for (const z of d.domains) { z.names ||= []; z.namesText = z.names.join(', ') }
    settings.ddns = d
    return settings
  }

  async function load() {
    try {
      const [settings, c] = await Promise.all([api.get('/settings'), api.get('/dns-credentials').catch(() => [])])
      s = ensure(settings)
      creds = c
      if (s.ddns.enabled) check()
    } catch (e) { toastErr(e) }
  }
  onMount(() => { loadMeta().catch(() => {}); load() })

  async function check() {
    checking = true
    try { status = await api.get('/ddns/status') } catch (e) { toastErr(e) }
    checking = false
  }

  async function save() {
    error = ''
    busy = true
    try {
      const body = JSON.parse(JSON.stringify(s))
      body.ddns.domains = body.ddns.domains.map((z) => ({ zone: (z.zone || '').trim(), names: (z.namesText || '').split(',').map((x) => x.trim()).filter(Boolean) })).filter((z) => z.zone)
      body.ddns.ip_sources = body.ddns.ip_sources.filter((x) => x.kind === 'upnp' || (x.value || '').trim())
      body.ddns.credential_id = Number(body.ddns.credential_id) || null
      delete body.acme_eab_mac_set; delete body.crowdsec_api_key_set
      s = ensure(await api.put('/settings', body))
      toast('Saved. Apply to update Caddy.')
      refreshStatus()
      if (s.ddns.enabled) check()
    } catch (e) { error = e.message }
    busy = false
  }

  const feature = $derived((app.meta?.features || []).find((f) => f.id === 'dynamic_dns'))
  const provider = $derived(creds.find((c) => c.id === Number(s?.ddns?.credential_id)))
  const providerMeta = $derived((app.meta?.dns_providers || []).find((p) => p.id === provider?.provider))
  const ro = $derived(!can.admin())
  const kinds = [['simple_http', 'Ask a website (simple_http)'], ['upnp', 'Ask the router (UPnP)'], ['interface', 'Network interface'], ['static', 'Fixed address']]
  const placeholder = { simple_http: 'https://icanhazip.com', interface: 'eth0', static: '203.0.113.5', upnp: '' }
</script>

<div class="page-head">
  <div>
    <h1>Dynamic DNS</h1>
    <div class="sub">Keep your domain pointed at this server when your public IP changes. Caddy checks the address on a schedule and updates the A/AAAA records through your DNS provider's API, using the same accounts as DNS validation.</div>
  </div>
  <button class="btn right" onclick={check} disabled={checking}><RefreshCw size={15} class={checking ? 'spin' : ''} /> Check now</button>
</div>

{#if s}
  <div class="stack">
    {#if feature && !feature.available}
      <div class="alert warn"><TriangleAlert /><span>The <code>caddy-dynamicdns</code> plugin isn't in your Caddy build. <a href="#/plugins">Install it under Plugins</a>; until then these settings are saved but skipped.</span></div>
    {/if}
    {#if provider && providerMeta && !providerMeta.available}
      <div class="alert warn"><TriangleAlert /><span>The {providerMeta.name} DNS plugin isn't installed. <a href="#/plugins">Add it under Plugins</a>.</span></div>
    {/if}

    {#if status}
      <div class="card stack">
        <div class="card-head"><span class="h-ic"><Signal size={18} /></span><div><h2>Status</h2><div class="desc">Checked {when(status.checked_at)} · public address from {status.source}</div></div></div>
        <div class="grid2">
          <div class="field"><span>Public IPv4</span><div class="mono">{status.public_ipv4 || '— (not reachable over IPv4)'}</div></div>
          <div class="field"><span>Public IPv6</span><div class="mono">{status.public_ipv6 || '—'}</div></div>
        </div>
        {#if status.records.length}
          <div class="table-wrap"><table class="list">
            <thead><tr><th>Name</th><th>Resolves to</th><th>State</th></tr></thead>
            <tbody>
              {#each status.records as r}
                <tr>
                  <td class="mono">{r.name}</td>
                  <td class="mono small">{r.addresses.join(', ') || '—'}</td>
                  <td>
                    {#if r.up_to_date === true}<span class="badge ok"><CircleCheck /> up to date</span>
                    {:else if r.up_to_date === false}<span class="badge warn"><CircleX /> differs</span>
                    {:else}<span class="badge">unknown</span>{/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table></div>
          <p class="muted small">“Differs” right after a change is normal: Caddy updates on the next check interval, and DNS caches take a while. The names are resolved from the panel's network, which may differ from the public internet.</p>
        {/if}
      </div>
    {/if}

    <fieldset class="stack" disabled={ro}>
      <div class="card stack">
        <label class="check"><input type="checkbox" bind:checked={s.ddns.enabled} /> Enable Dynamic DNS</label>
        {#if s.ddns.enabled}
          <label class="field"><span>DNS provider account</span>
            <select bind:value={s.ddns.credential_id}>
              <option value={null}>Choose…</option>
              {#each creds as c}<option value={c.id}>{c.name}</option>{/each}
            </select>
            {#if creds.length === 0}<span class="hint">Add an account under <a href="#/domains">Domains &amp; DNS</a> first. The token needs permission to edit A/AAAA records.</span>{/if}
          </label>

          <div class="field">
            <span>Records to keep updated</span>
            {#each s.ddns.domains as z, i}
              <div class="row">
                <input type="text" bind:value={z.zone} placeholder="example.com" style="max-width:240px" />
                <input type="text" bind:value={z.namesText} placeholder="@, www, home" class="grow" />
                <button class="btn ghost icon sm" type="button" aria-label="Remove" onclick={() => s.ddns.domains.splice(i, 1)}><Trash2 size={15} /></button>
              </div>
            {/each}
            <div><button class="btn xs" type="button" onclick={() => s.ddns.domains.push({ zone: '', names: [], namesText: '@' })}><Plus size={14} /> Add zone</button></div>
            <span class="hint">The zone, then the names inside it separated by commas: <code>@</code> is the zone itself, <code>www</code> becomes www.example.com.</span>
          </div>
          <label class="check"><input type="checkbox" bind:checked={s.ddns.dynamic_domains} /> Also update every hostname served by Caddy that belongs to these zones</label>

          <div class="field">
            <span>How to find the public address (first one that works)</span>
            {#each s.ddns.ip_sources as src, i}
              <div class="row">
                <select bind:value={src.kind} style="max-width:260px">{#each kinds as [k, l]}<option value={k}>{l}</option>{/each}</select>
                {#if src.kind !== 'upnp'}<input type="text" bind:value={src.value} placeholder={placeholder[src.kind]} class="grow" />{/if}
                <button class="btn ghost icon sm" type="button" aria-label="Remove" onclick={() => s.ddns.ip_sources.splice(i, 1)}><Trash2 size={15} /></button>
              </div>
            {/each}
            <div><button class="btn xs" type="button" onclick={() => s.ddns.ip_sources.push({ kind: 'simple_http', value: 'https://icanhazip.com' })}><Plus size={14} /> Add source</button></div>
            <span class="hint">Empty = Caddy's default (asks icanhazip.com). Use UPnP if your router supports it, or an interface name when the server has a public address itself.</span>
          </div>

          <div class="grid2">
            <label class="field"><span>Check every</span><input type="text" bind:value={s.ddns.check_interval} placeholder="5m" /></label>
            <label class="field"><span>Record TTL</span><input type="text" bind:value={s.ddns.ttl} placeholder="provider default (e.g. 1h)" /></label>
          </div>
          <div class="row">
            <label class="check"><input type="checkbox" bind:checked={s.ddns.ipv4} /> IPv4 (A records)</label>
            <label class="check"><input type="checkbox" bind:checked={s.ddns.ipv6} /> IPv6 (AAAA records)</label>
            <label class="check"><input type="checkbox" bind:checked={s.ddns.update_only} /> Only update records that already exist</label>
          </div>
        {/if}
      </div>
    </fieldset>

    <div class="alert info small"><Info /><span>Dynamic DNS keeps your records correct; HTTPS certificates still come from the domain's validation settings. For a home connection, DNS validation under Domains &amp; DNS avoids opening port 80.</span></div>

    {#if error}<ErrorBox message={error} />{/if}
    {#if can.admin()}<div class="row"><button class="btn primary" onclick={save} disabled={busy}>Save</button></div>{/if}
  </div>
{:else}
  <p class="muted">Loading…</p>
{/if}

<style>
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  .h-ic { width: 36px; height: 36px; border-radius: 10px; display: grid; place-items: center; background: var(--accent-soft); color: var(--accent-soft-text); flex: none; }
  :global(.spin) { animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
