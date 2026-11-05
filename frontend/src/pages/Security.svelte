<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { onMount, onDestroy } from 'svelte'
  import { Shield, ShieldCheck, Bot, EyeOff, Gauge, House, KeyRound, RefreshCw, Download, Power, Trash2, Plus, CircleCheck, CircleX, TriangleAlert, Info, ChevronDown, Rocket, LogOut, Ban } from '@lucide/svelte'
  import { api } from '../lib/api.js'
  import { toast, toastErr, refreshStatus, can } from '../lib/state.svelte.js'
  import { when } from '../lib/util.js'
  import Switch from '../lib/Switch.svelte'
  import ListInput from '../lib/ListInput.svelte'

  const BOUNCER = 'github.com/hslatman/caddy-crowdsec-bouncer'
  let d = $state(null)        // GET /security
  let sec = $state(null)      // editable copy of d.security
  let ext = $state(null)      // external CrowdSec
  let busy = $state(false)
  let error = $state('')
  let decisions = $state([])
  let sessions = $state([])
  let banIp = $state('')
  let banFor = $state('24h')
  let showLog = $state(false)
  let showExt = $state(false)
  let timer = null

  function take(data) {
    d = data
    const s = JSON.parse(JSON.stringify(data.security))
    s.home_networks ||= []
    s.bots_allow ||= []
    s.basic_auth_users = (s.basic_auth_users || []).map((u) => ({ username: u.username, password: '', saved: true }))
    sec = s
    ext = { enabled: data.crowdsec_external.enabled, api_url: data.crowdsec_external.api_url, api_key: '' }
    if (data.crowdsec?.job?.status === 'running') watch()
  }
  async function load() {
    try { take(await api.get('/security')) } catch (e) { toastErr(e) }
    loadDecisions(); loadSessions()
  }
  async function loadDecisions() {
    if (!d?.crowdsec?.running) { decisions = []; return }
    try { decisions = await api.get('/crowdsec/decisions') } catch { decisions = [] }
  }
  async function loadSessions() {
    try { sessions = await api.get('/security/sessions') } catch { sessions = [] }
  }
  function watch() {
    if (timer) return
    timer = setInterval(async () => {
      try {
        const cs = await api.get('/crowdsec')
        if (d) d.crowdsec = cs
        if (!cs.job || cs.job.status !== 'running') {
          clearInterval(timer); timer = null
          if (cs.job?.status === 'failed') toast('CrowdSec: ' + cs.job.error, 'err', 9000)
          else toast('CrowdSec is ready.')
          load(); refreshStatus()
        }
      } catch { clearInterval(timer); timer = null }
    }, 2500)
  }
  onMount(load)
  onDestroy(() => clearInterval(timer))

  async function save() {
    error = ''
    busy = true
    try {
      const body = JSON.parse(JSON.stringify(sec))
      body.home_networks = body.home_networks.map((x) => x.trim()).filter(Boolean)
      body.bots_allow = body.bots_allow.map((x) => x.trim()).filter(Boolean)
      body.basic_auth_users = body.basic_auth_users.filter((u) => u.username.trim()).map((u) => ({ username: u.username.trim(), password: u.password }))
      body.rate_limit_events = Number(body.rate_limit_events) || 600
      body.sso.session_hours = Number(body.sso.session_hours) || 168
      take(await api.put('/security', { security: body, crowdsec_external: ext }))
      toast('Saved. Apply on Review & apply to update Caddy.')
      refreshStatus()
    } catch (e) { error = e.message }
    busy = false
  }

  // Per-site overrides: null = follow the default.
  async function setSite(site, patch) {
    const o = { ...site.overrides, ...patch }
    for (const k of Object.keys(o)) if (o[k] === null || o[k] === undefined) delete o[k]
    try { take(await api.put(`/security/sites/${site.id}`, o)); refreshStatus() } catch (e) { toastErr(e) }
  }
  function cycle(site, key) {
    const cur = site.overrides[key]
    const next = cur === undefined || cur === null ? !site.effective[key] : cur === !sec.defaults[key] ? null : !cur
    setSite(site, { [key]: next })
  }

  // Suggestions: protections a newer version recommends, never switched on
  // for existing sites by the update itself.
  let pick = $state({})
  async function suggest(sg, siteId) {
    busy = true
    try {
      take(await api.post(`/security/suggestions/${sg.id}/apply`, siteId ? { site_id: Number(siteId) } : {}))
      toast(siteId ? 'Turned on for that site. Apply on Review & apply to update Caddy.' : 'Turned on for all sites. Apply on Review & apply to update Caddy.')
      refreshStatus()
    } catch (e) { toastErr(e) }
    busy = false
  }
  async function dismissSuggestion(sg) {
    try { take(await api.post(`/security/suggestions/${sg.id}/dismiss`)) } catch (e) { toastErr(e) }
  }

  async function crowdsec(action) {
    busy = true
    try {
      const cs = await api.post('/crowdsec/' + action)
      d.crowdsec = cs
      if (cs.job?.status === 'running') { showLog = true; watch() } else load()
      if (action === 'disable') toast('CrowdSec turned off.')
    } catch (e) { toastErr(e) }
    busy = false
  }
  async function installBouncer() {
    if (!confirm('Download a Caddy build that adds the CrowdSec bouncer (keeps your other plugins) and restart Caddy? Sites are down for a few seconds.')) return
    try { await api.post('/plugins/install', { add: [BOUNCER], remove: [] }); toast('Building Caddy with the bouncer; follow it on the Plugins page.') } catch (e) { toastErr(e) }
  }
  async function ban() {
    try { decisions = await api.post('/crowdsec/decisions', { ip: banIp.trim(), duration: banFor }); banIp = ''; toast('Banned.') } catch (e) { toastErr(e) }
  }
  async function unban(x) {
    try { decisions = await api.del(`/crowdsec/decisions/${x.id}`); toast(`Unbanned ${x.value}.`) } catch (e) { toastErr(e) }
  }
  async function refreshBots() {
    busy = true
    try { const r = await api.post('/security/bots/refresh'); toast(r.changed ? `List updated: ${r.ai_count} AI crawlers.` : `Already current: ${r.ai_count} AI crawlers.`); load(); refreshStatus() } catch (e) { toastErr(e) }
    busy = false
  }
  async function endSessions(id) {
    if (id === 'all' && !confirm('Sign everyone out of the protected sites?')) return
    try { await api.del(`/security/sessions/${id}`); loadSessions(); load(); toast('Signed out.') } catch (e) { toastErr(e) }
  }

  const loginLabel = { '': 'No login', basic: 'Password', sso: 'Sign-in + 2FA' }
  const cols = [
    ['crowdsec', 'CrowdSec', Shield],
    ['bots', 'Bots & AI', Bot],
    ['privacy', 'No-index', EyeOff],
    ['rate_limit', 'Rate limit', Gauge],
    ['home_only', 'Home only', House],
  ]
  const cs = $derived(d?.crowdsec)
  const csReady = $derived(d && d.crowdsec.running && d.plugins.crowdsec)
  const ssoUsed = $derived(d && d.sites.some((s) => s.enabled && s.effective.login === 'sso'))
  const members = $derived(d ? d.users.filter((u) => !u.disabled) : [])
  const vis = $derived(d?.visitors)
  const suggestPortal = $derived.by(() => {
    const h = d?.sites?.find((s) => s.hosts?.length)?.hosts[0] || ''
    const parts = h.split('.')
    return parts.length > 2 ? 'auth.' + parts.slice(-2).join('.') : h ? 'auth.' + h : 'auth.example.com'
  })
</script>

<div class="page-head">
  <div>
    <h1>Security</h1>
    <div class="sub">Protections for every site at once. Each site can turn any of them on or off for itself.</div>
  </div>
  {#if d?.pending_changes && can.edit()}<a class="btn primary right" href="#/changes"><Rocket size={15} /> Review & apply</a>{/if}
</div>

{#if !d || !sec}
  <p class="muted">Loading…</p>
{:else}
  <div class="stack">
    {#if d.suggestions?.length && can.admin()}
      <div class="card stack suggestions">
        <div class="card-head"><div><h2>Suggested protections</h2><div class="desc">Newer versions recommend these. Updates never switch them on for sites you already have; turn them on here if you want them.</div></div></div>
        {#each d.suggestions as sg (sg.id)}
          <div class="suggestion">
            <div class="grow"><strong>{sg.title}</strong><div class="hint">{sg.body} Not on for {sg.sites.length === 1 ? sg.sites[0].name : `${sg.sites.length} sites`}.</div></div>
            <div class="row wrap">
              <button class="btn primary sm" disabled={busy} onclick={() => suggest(sg)}>Turn on for all sites</button>
              {#if sg.sites.length > 1}
                <select bind:value={pick[sg.id]} aria-label="Site">
                  <option value={undefined}>One site…</option>
                  {#each sg.sites as s}<option value={s.id}>{s.name}</option>{/each}
                </select>
                <button class="btn sm" disabled={busy || !pick[sg.id]} onclick={() => suggest(sg, pick[sg.id])}>Turn on</button>
              {/if}
              <button class="btn ghost sm" onclick={() => dismissSuggestion(sg)}>Dismiss</button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
    <!-- Defaults -->
    <div class="card stack">
      <div class="card-head"><div><h2>Protection for all sites</h2><div class="desc">What every site gets unless it says otherwise below.</div></div></div>
      <div class="opts">
        <label class="opt">
          <Switch bind:checked={sec.defaults.crowdsec} label="CrowdSec" />
          <div><strong>Block known attackers</strong><div class="hint">CrowdSec bans IPs that scan, brute-force or exploit, plus a community blocklist. {#if !csReady}<span class="warn-text">Not active yet: set it up below.</span>{/if}</div></div>
        </label>
        <label class="opt">
          <Switch bind:checked={sec.defaults.bots} label="Bots" />
          <div><strong>Block AI crawlers and scrapers</strong><div class="hint">{d.bots.ai_count} known AI crawlers (GPTBot, ClaudeBot, Bytespider, Perplexity…), refreshed daily.</div>
            {#if sec.defaults.bots}<label class="check sub-opt"><input type="checkbox" bind:checked={sec.bots_block_all} /> Also block search engines, SEO tools, archivers and link previews ({d.bots.other_count} more)</label>{/if}</div>
        </label>
        <label class="opt">
          <Switch bind:checked={sec.defaults.privacy} label="No-index" />
          <div><strong>Keep out of search engines and AI datasets</strong><div class="hint">Serves a robots.txt that disallows everything, sends noindex/noai headers and hides the server name.</div></div>
        </label>
        <label class="opt">
          <Switch bind:checked={sec.defaults.rate_limit} label="Rate limit" />
          <div><strong>Limit requests per visitor</strong><div class="hint">Slows down password guessing and scraping. {#if !d.plugins.rate_limit}<span class="warn-text">Needs the rate-limit plugin.</span>{/if}</div>
            {#if sec.defaults.rate_limit}
              <div class="inline sub-opt"><input type="number" min="10" bind:value={sec.rate_limit_events} style="width:90px" /> requests per <input type="text" bind:value={sec.rate_limit_window} style="width:70px" /></div>
              <label class="check sub-opt"><input type="checkbox" bind:checked={sec.rate_limit_exempt_home} /> Never limit devices at home</label>
            {/if}</div>
        </label>
        <label class="opt">
          <Switch bind:checked={sec.defaults.home_only} label="Home only" />
          <div><strong>Home only</strong><div class="hint">Only devices on your home network{sec.tailscale_is_home ? ' or Tailscale' : ''} get in. Best for admin pages; rarely right for every site.</div></div>
        </label>
        <div class="opt">
          <span class="ic"><KeyRound size={18} /></span>
          <div class="grow"><strong>Login wall</strong><div class="hint">Asked before the site's own login. Phone and TV apps can't pass a login wall; pick the app per site below to leave their paths open.</div>
            <select bind:value={sec.defaults.login} class="sub-opt" style="max-width:260px">
              <option value="">No login wall</option>
              <option value="sso">Sign-in with two-factor</option>
              <option value="basic">Shared password</option>
            </select></div>
        </div>
      </div>
      {#if error}<ErrorBox message={error} />{/if}
      {#if can.admin()}<div class="row"><button class="btn primary" onclick={save} disabled={busy}>Save</button></div>{/if}
    </div>

    <!-- Per site -->
    <div class="card stack">
      <div class="card-head"><div><h2>Per site</h2><div class="desc">Click a switch to override it for one site; click again to go back to the default (shown faded).</div></div></div>
      {#if d.sites.length === 0}
        <p class="muted">No sites yet.</p>
      {:else}
        <div class="table-wrap"><table class="list matrix">
          <thead><tr><th>Site</th>{#each cols as [, l]}<th>{l}</th>{/each}<th>Login</th><th>Apps</th></tr></thead>
          <tbody>
            {#each d.sites as s (s.id)}
              <tr class:off={!s.enabled}>
                <td><a href={'#/sites/' + s.id}><strong>{s.name || s.hosts[0]}</strong></a><div class="muted xs truncate">{s.hosts.join(', ')}</div></td>
                {#each cols as [k]}
                  {@const v = s.effective[k]}
                  {@const own = s.overrides[k] !== undefined && s.overrides[k] !== null}
                  <td><button class="pill" class:on={v} class:inherit={!own} disabled={!can.admin()} onclick={() => cycle(s, k)} title={own ? 'Set for this site (click to change)' : 'Default (click to override)'}>{v ? 'On' : 'Off'}</button></td>
                {/each}
                <td>
                  <select class="mini" value={s.overrides.login ?? '__default'} disabled={!can.admin()} onchange={(e) => setSite(s, { login: e.target.value === '__default' ? null : e.target.value })}>
                    <option value="__default">Default ({loginLabel[sec.defaults.login]})</option>
                    <option value="">No login</option>
                    <option value="sso">Sign-in + 2FA</option>
                    <option value="basic">Password</option>
                  </select>
                </td>
                <td>
                  <select class="mini" value={s.overrides.apps || ''} disabled={!can.admin() || !s.effective.login} onchange={(e) => setSite(s, { apps: e.target.value })} title={s.effective.login ? 'Which app paths skip the login wall' : 'Only matters with a login wall'}>
                    <option value="">Browser only</option>
                    {#each d.app_presets as p}<option value={p.id}>{p.name}</option>{/each}
                    {#if s.overrides.apps === 'custom'}<option value="custom">Custom paths</option>{/if}
                  </select>
                </td>
              </tr>
            {/each}
          </tbody>
        </table></div>
        <p class="hint">Per-site extras (allowed sign-in accounts, custom app paths, extra IP rules) are in each site's editor under Access.</p>
      {/if}
    </div>

    <!-- Sign-in -->
    <div class="card stack">
      <div class="card-head">
        <div><h2>Sign-in with two-factor</h2><div class="desc">One sign-in page for all your sites, using Caddymium accounts. Signing in once covers every protected site for the session length.</div></div>
      </div>
      <div class="grid2">
        <label class="field"><span>Sign-in page address</span><input type="text" bind:value={sec.sso.portal_host} placeholder={suggestPortal} />
          <span class="hint">A new name under your domain, e.g. {suggestPortal}. Caddy serves it with HTTPS; Dynamic DNS keeps it pointed home.</span></label>
        <label class="field"><span>Stay signed in for</span>
          <select bind:value={sec.sso.session_hours}>
            <option value={12}>12 hours</option><option value={24}>1 day</option><option value={168}>7 days</option><option value={720}>30 days</option><option value={2160}>90 days</option>
          </select></label>
      </div>
      <label class="check"><input type="checkbox" bind:checked={sec.sso.require_2fa} /> Require a code from an authenticator app (accounts without one set it up at their first sign-in)</label>
      {#if ssoUsed && !sec.sso.portal_host}<div class="alert warn"><TriangleAlert /><span>Some sites use sign-in, but there's no sign-in page address yet. Those sites stay closed until you set one and apply.</span></div>{/if}
      <div>
        <div class="small"><strong>Who can sign in:</strong> every active Caddymium account ({members.map((u) => u.username + (u.totp_enabled ? ' ✓2FA' : '')).join(', ') || 'none'}). Add family or friends as <strong>Member</strong> on <a href="#/users">Users</a>: they can use your sites but not this panel. Limit a site to certain accounts in its editor.</div>
      </div>
      <div class="row tight">
        <span class="small muted">{d.sso.sessions} active sign-in(s){sec.sso.portal_host ? ` · cookie for ${d.sso.cookie_domain}` : ''}</span>
        {#if d.sso.sessions && can.admin()}<button class="btn sm right" onclick={() => endSessions('all')}><LogOut size={14} /> Sign everyone out</button>{/if}
      </div>
      {#if sessions.length}
        <div class="table-wrap"><table class="list">
          <thead><tr><th>Account</th><th>From</th><th class="hide-sm">Device</th><th>Last seen</th><th></th></tr></thead>
          <tbody>{#each sessions as x}<tr><td>{x.username}{#if x.mfa} <span class="badge ok">2FA</span>{/if}</td><td class="mono small">{x.ip}</td><td class="hide-sm small truncate" style="max-width:220px">{x.user_agent}</td><td class="small nowrap">{when(x.last_seen)}</td><td style="text-align:right">{#if can.admin()}<button class="btn xs" onclick={() => endSessions(x.id)}>Sign out</button>{/if}</td></tr>{/each}</tbody>
        </table></div>
      {/if}
      {#if can.admin()}<div class="row"><button class="btn primary" onclick={save} disabled={busy}>Save</button></div>{/if}
    </div>

    <!-- Shared password accounts -->
    <div class="card stack">
      <div class="card-head"><div><h2>Shared password accounts</h2><div class="desc">For the simpler "Shared password" login wall: the browser asks for a name and password. No two-factor; prefer sign-in where you can.</div></div></div>
      {#each sec.basic_auth_users as u, i}
        <div class="row tight">
          <input type="text" bind:value={u.username} placeholder="name" style="max-width:180px" />
          <input type="password" bind:value={u.password} placeholder={u.saved ? '•••••• (unchanged)' : 'password (8+ characters)'} autocomplete="new-password" />
          <button class="btn ghost icon sm" aria-label="Remove account" onclick={() => sec.basic_auth_users.splice(i, 1)}><Trash2 size={15} /></button>
        </div>
      {/each}
      <div class="row"><button class="btn sm" onclick={() => sec.basic_auth_users.push({ username: '', password: '', saved: false })}><Plus size={14} /> Add account</button>
        {#if can.admin()}<button class="btn primary sm" onclick={save} disabled={busy}>Save</button>{/if}</div>
    </div>

    <!-- Home -->
    <div class="card stack">
      <div class="card-head"><div><h2>Home networks</h2><div class="desc">Used by "Home only" and to keep home devices out of rate limits.</div></div></div>
      <ListInput bind:items={sec.home_networks} placeholder="10.0.1.0/24" addLabel="Add network" />
      <label class="check"><input type="checkbox" bind:checked={sec.tailscale_is_home} /> Count Tailscale devices as home (100.64.0.0/10)</label>
      {#if vis?.available}
        {#if vis.public > 0}
          <div class="alert ok"><CircleCheck /><span>Real visitor addresses reach Caddy ({vis.public} recent requests from the internet, {vis.home} from home), so home-only and rate limits can tell visitors apart.</span></div>
        {:else if vis.home > 0 && vis.docker_gateway > vis.home / 2}
          <div class="alert warn"><TriangleAlert /><span>Every recent visitor appears to come from a Docker address, so home-only would let everyone in and rate limits would treat everyone as one visitor. This happens when Docker's userland proxy hides real addresses; publish the ports with the default iptables setup.</span></div>
        {/if}
      {/if}
      {#if can.admin()}<div class="row"><button class="btn primary sm" onclick={save} disabled={busy}>Save</button></div>{/if}
    </div>

    <!-- Bots -->
    <div class="card stack">
      <div class="card-head">
        <div><h2>Crawler list</h2><div class="desc">AI crawler names come from the community-maintained ai.robots.txt list, refreshed daily. {d.bots.fetched_at ? 'Last refreshed ' + when(d.bots.fetched_at) + '.' : 'Using the built-in copy until the first refresh.'}</div></div>
        {#if can.admin()}<button class="btn ghost sm right" onclick={refreshBots} disabled={busy}><RefreshCw size={14} /> Refresh now</button>{/if}
      </div>
      <label class="field"><span>Never block these user agents</span>
        <ListInput bind:items={sec.bots_allow} placeholder="Uptime-Kuma" addLabel="Add exception" />
        <span class="hint">Part of a user-agent string, e.g. your uptime monitor. Your own apps don't need this: they don't look like crawlers.</span></label>
      {#if can.admin()}<div class="row"><button class="btn primary sm" onclick={save} disabled={busy}>Save</button></div>{/if}
    </div>

    <!-- CrowdSec -->

    <div class="card stack">
      <div class="card-head">
        <span class="s-ic" class:on={cs.running}><ShieldCheck size={18} /></span>
        <div class="grow">
          <h2>CrowdSec</h2>
          <div class="desc">
            {#if !cs.supported}{cs.reason}
            {:else if cs.running}Running {cs.installed_version}{cs.started_at ? ' since ' + when(cs.started_at) : ''} · rules updated {cs.hub_updated_at ? when(cs.hub_updated_at) : 'never'}
            {:else if cs.enabled}Starting…
            {:else}Built in and off. Turning it on downloads CrowdSec (about 85 MB) and sets it up; no extra container needed.{/if}
          </div>
        </div>
        {#if can.admin() && cs.supported}
          {#if cs.enabled}
            {#if cs.update_available}<button class="btn sm" onclick={() => crowdsec('update')} disabled={busy || cs.job?.status === 'running'}><Download size={14} /> Update to {cs.latest_version}</button>{/if}
            <button class="btn sm" onclick={() => crowdsec('rules')} disabled={busy || cs.job?.status === 'running'} title="Refresh the detection rules now"><RefreshCw size={14} /> Rules</button>
            <button class="btn sm danger" onclick={() => crowdsec('disable')} disabled={busy}><Power size={14} /> Turn off</button>
          {:else}
            <button class="btn primary sm" onclick={() => crowdsec('enable')} disabled={busy || cs.job?.status === 'running'}><Power size={14} /> Turn on</button>
          {/if}
        {/if}
      </div>
      {#if cs.enabled && !d.plugins.crowdsec}
        <div class="alert warn"><TriangleAlert /><span>Caddy doesn't have the CrowdSec bouncer plugin yet, so bans aren't enforced. {#if can.admin()}<button class="btn xs" onclick={installBouncer}>Install it</button>{/if}</span></div>
      {/if}
      {#if cs.last_auto && !cs.last_auto.ok}<ErrorBox title={`Automatic update to ${cs.last_auto.to} failed`} message={cs.last_auto.error} />{/if}
      {#if cs.job}
        <div class="row tight small"><span class="badge {cs.job.status === 'failed' ? 'err' : cs.job.status === 'done' ? 'ok' : 'accent'}">{cs.job.kind}: {cs.job.status}</span>
          <button class="btn ghost xs" onclick={() => (showLog = !showLog)}><ChevronDown size={13} /> {showLog ? 'Hide' : 'Show'} log</button></div>
        {#if cs.job.status === 'failed'}<ErrorBox message={cs.job.error} />{/if}
        {#if showLog}<pre class="log">{cs.job.log.join('\n')}</pre>{/if}
      {/if}
      {#if cs.running}
        <h3 class="sub-h">Banned right now</h3>
        {#if decisions.length === 0}
          <p class="muted small">Nobody is banned by your CrowdSec at the moment. The community blocklist is applied on top and isn't listed here.</p>
        {:else}
          <div class="table-wrap"><table class="list">
            <thead><tr><th>Address</th><th>Why</th><th class="hide-sm">Until</th><th></th></tr></thead>
            <tbody>{#each decisions as x}<tr><td class="mono small">{x.value}</td><td class="small">{x.reason.replace('crowdsecurity/', '')}</td><td class="hide-sm small">{x.until}</td><td style="text-align:right">{#if can.admin()}<button class="btn xs" onclick={() => unban(x)}>Unban</button>{/if}</td></tr>{/each}</tbody>
          </table></div>
        {/if}
        {#if can.admin()}
          <div class="row tight">
            <input type="text" bind:value={banIp} placeholder="IP or range to ban" style="max-width:220px" />
            <select bind:value={banFor} style="max-width:120px"><option value="4h">4 hours</option><option value="24h">1 day</option><option value="168h">1 week</option><option value="8760h">1 year</option></select>
            <button class="btn sm" onclick={ban} disabled={!banIp.trim()}><Ban size={14} /> Ban</button>
          </div>
        {/if}
        {#if cs.log_tail?.length}
          <details><summary class="small muted">Engine log</summary><pre class="log">{cs.log_tail.join('\n')}</pre></details>
        {/if}
      {/if}
      <details bind:open={showExt}>
        <summary class="small muted">Use an external CrowdSec instead</summary>
        <div class="stack" style="margin-top:10px">
          <label class="check"><input type="checkbox" bind:checked={ext.enabled} /> Use a CrowdSec running elsewhere (ignored while the built-in one is on)</label>
          {#if ext.enabled}
            <div class="grid2">
              <label class="field"><span>Local API URL</span><input type="text" bind:value={ext.api_url} placeholder="http://crowdsec:8080" /></label>
              <label class="field"><span>Bouncer API key</span><input type="password" bind:value={ext.api_key} placeholder={d.crowdsec_external.key_set ? '•••••• (saved)' : ''} autocomplete="off" /></label>
            </div>
          {/if}
          {#if can.admin()}<div class="row"><button class="btn primary sm" onclick={save} disabled={busy}>Save</button></div>{/if}
        </div>
      </details>
    </div>
  </div>
{/if}

<style>
  .suggestion { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; padding-top: 10px; border-top: 1px solid var(--border); }
  .suggestion .grow { flex: 1 1 260px; }
  .suggestion .row { flex: 0 1 auto; align-items: center; gap: 8px; }
  .suggestion select { width: auto; max-width: 200px; }
  .opts { display: grid; gap: 14px; }
  .opt { display: flex; gap: 12px; align-items: flex-start; cursor: pointer; }
  .opt :global(.switch) { margin-top: 2px; }
  .opt .ic { width: 38px; display: grid; place-items: center; color: var(--muted); flex: none; }
  .sub-opt { margin-top: 8px; }
  .inline { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 13.5px; }
  .warn-text { color: var(--warn-soft-text); }
  .matrix th, .matrix td { white-space: nowrap; }
  .matrix td:first-child { white-space: normal; min-width: 150px; }
  .pill { font: inherit; font-size: 12px; font-weight: 650; padding: 3px 10px; border-radius: 999px; border: 1px solid var(--border); background: var(--surface-2); color: var(--muted); cursor: pointer; min-width: 44px; }
  .pill.on { background: var(--accent-soft); color: var(--accent-soft-text); border-color: transparent; }
  .pill.inherit { opacity: 0.55; border-style: dashed; }
  .pill:disabled { cursor: default; }
  select.mini { font-size: 12.5px; padding: 4px 8px; min-width: 120px; }
  tr.off td { opacity: 0.5; }
  .s-ic { width: 36px; height: 36px; border-radius: 10px; display: grid; place-items: center; background: var(--surface-2); color: var(--muted); flex: none; }
  .s-ic.on { background: var(--ok-soft); color: var(--ok); }
  .sub-h { font-size: 14px; margin: 6px 0 0; }
  .log { max-height: 280px; overflow: auto; font-size: 12px; }
  details summary { cursor: pointer; }
  .row.tight { gap: 8px; flex-wrap: wrap; align-items: center; }
</style>
