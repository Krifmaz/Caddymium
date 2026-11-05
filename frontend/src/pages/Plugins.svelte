<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { onMount, onDestroy } from 'svelte'
  import { Puzzle, Search, Plus, Minus, Download, RefreshCw, Undo2, Trash2, CircleCheck, CircleX, TriangleAlert, Info, ChevronDown, ExternalLink } from '@lucide/svelte'
  import { api } from '../lib/api.js'
  import { app, toast, toastErr, refreshStatus, can } from '../lib/state.svelte.js'
  import { when } from '../lib/util.js'

  let st = $state(null)
  let upd = $state(null)   // /updates: checker state and schedule
  let us = $state(null)    // /settings (the updates part is edited here)
  let q = $state('')
  let cat = $state('featured')
  let add = $state([])
  let remove = $state([])
  let busy = $state(false)
  let showLog = $state(true)
  let timer = null

  async function load() {
    try { st = await api.get('/plugins') } catch (e) { toastErr(e) }
    try {
      upd = await api.get('/updates')
      if (can.admin()) us = await api.get('/settings')
    } catch (e) { toastErr(e) }
  }
  async function checkNow() {
    busy = true
    try {
      upd = await api.post('/updates/check')
      if (upd.last_error) toast(upd.last_error, 'err', 7000)
      else toast(upd.update_available ? `Caddy ${upd.latest} is available` : `Caddy ${upd.running || ''} is up to date`)
      st = await api.get('/plugins'); refreshStatus()
    } catch (e) { toastErr(e) }
    busy = false
  }
  async function saveUpdates() {
    busy = true
    try {
      us = await api.put('/settings', us)
      upd = await api.get('/updates')
      toast('Update settings saved'); refreshStatus()
    } catch (e) { toastErr(e) }
    busy = false
  }
  const intervals = [[1, 'hour'], [6, '6 hours'], [12, '12 hours'], [24, 'day'], [168, 'week']]
  async function poll() {
    try {
      const j = await api.get('/plugins/job')
      if (st) st.job = j
      if (j && j.status !== 'running') {
        clearInterval(timer); timer = null
        if (j.status === 'done') { toast('Done. Caddy is running the new build.'); add = []; remove = [] }
        else toast('The plugin operation failed; see the log.', 'err', 8000)
        app.meta = null
        load(); refreshStatus()
      }
    } catch (e) { clearInterval(timer); timer = null }
  }
  function watch() { if (!timer) timer = setInterval(poll, 2000) }
  onMount(async () => { await load(); if (st?.job?.status === 'running') watch() })
  onDestroy(() => clearInterval(timer))

  const categories = [['featured', 'Featured'], ['dns', 'DNS providers'], ['security', 'Security'], ['proxy', 'Proxying'], ['network', 'Network'], ['other', 'Other'], ['all', 'All']]
  const installedPaths = $derived(new Set((st?.installed || []).map((i) => i.path)))
  const shown = $derived((st?.catalog || []).filter((p) => {
    if (cat === 'featured' && !p.featured) return false
    if (cat !== 'featured' && cat !== 'all' && p.category !== cat) return false
    if (!q) return true
    const hay = [p.path, p.name, p.description, ...(p.modules || [])].join(' ').toLowerCase()
    return hay.includes(q.toLowerCase())
  }))
  const pending = $derived(add.length + remove.length)
  const running = $derived(st?.job?.status === 'running')

  function toggleAdd(p) { add = add.includes(p) ? add.filter((x) => x !== p) : [...add, p] }
  function toggleRemove(p) { remove = remove.includes(p) ? remove.filter((x) => x !== p) : [...remove, p] }

  async function run(body, path = '/plugins/install') {
    busy = true
    try {
      const j = await api.post(path, body)
      if (st) st.job = j
      showLog = true
      watch()
    } catch (e) { toastErr(e) }
    busy = false
  }
  function applyChanges() {
    const n = add.length + remove.length
    if (!confirm(`Download a new Caddy build with ${n} change(s) and restart Caddy? Sites are unavailable for a few seconds during the restart.`)) return
    run({ add, remove })
  }
  function upgrade() {
    if (!confirm(`Rebuild Caddy ${st.latest_caddy || '(latest release)'} with the current plugins and restart it?`)) return
    run({ add: [], remove: [] })
  }
  function reset() {
    if (!confirm('Remove the custom build and go back to the Caddy that ships in the image? Plugins that are not in the image stop working.')) return
    run({}, '/plugins/reset')
  }
  function rollback() {
    if (!confirm('Switch back to the previous custom build and restart Caddy?')) return
    run({}, '/plugins/rollback')
  }
  function restart() {
    if (!confirm('Restart Caddy? Sites are unavailable for a few seconds. If Caddy does not answer it is stopped forcefully, and if it does not come back the image\'s built-in Caddy takes over.')) return
    run({}, '/plugins/restart')
  }
  async function refreshCatalog() {
    busy = true
    try { const c = await api.post('/plugins/catalog/refresh'); if (st) { st.catalog = c.packages; st.catalog_fetched_at = c.fetched_at }; toast('Catalog refreshed') } catch (e) { toastErr(e) }
    busy = false
  }
  const fmtDl = (n) => (n >= 1e6 ? (n / 1e6).toFixed(1) + 'M' : n >= 1e3 ? (n / 1e3).toFixed(0) + 'k' : String(n || 0))
</script>

<div class="page-head">
  <div>
    <h1>Plugins</h1>
    <div class="sub">Caddy plugins are compiled in. Caddymium fetches a build with your selection from the official download service (what <code>caddy add-package</code> uses), checks your config against it, then restarts Caddy. The previous build is kept so you can roll back.</div>
  </div>
  <button class="btn right" onclick={load} disabled={busy}><RefreshCw size={15} /> Reload</button>
</div>

{#if !st}
  <p class="muted">Loading…</p>
{:else}
  <div class="stack">
    {#if !st.supported}
      <div class="alert warn"><TriangleAlert /><span><strong>Installing is not available:</strong> {st.reason}. The panel needs write access to the shared <code>caddy-bin</code> volume (see the compose file) and a running Caddy.</span></div>
    {/if}

    <div class="card">
      <div class="status">
        <div class="s-item">
          <span class="s-ic"><Puzzle size={18} /></span>
          <div><div class="s-label">Caddy build</div><div class="s-val">{st.caddy_version || 'unknown'} <span class="faint small">{st.os}/{st.arch}</span></div></div>
        </div>
        <div class="s-item">
          <div>
            <div class="s-label">Source</div>
            <div class="s-val">{#if st.custom_binary}<span class="badge accent">custom build</span>{:else}<span class="badge">image build</span>{/if}</div>
          </div>
        </div>
        <div class="s-item">
          <div>
            <div class="s-label">Latest release</div>
            <div class="s-val">{st.latest_caddy || '—'} {#if st.update_available}<span class="badge warn">update available</span>{:else if st.latest_caddy}<span class="badge ok"><CircleCheck /> up to date</span>{/if}</div>
          </div>
        </div>
        {#if can.admin()}
          <div class="s-actions">
            <button class="btn sm" onclick={upgrade} disabled={busy || running || !st.supported} title="Rebuild with the current plugins against the latest Caddy release"><Download size={14} /> {st.update_available ? 'Update Caddy' : 'Rebuild latest'}</button>
            <button class="btn sm" onclick={restart} disabled={busy || running}><RefreshCw size={14} /> Restart Caddy</button>
            {#if st.previous_binary}<button class="btn sm" onclick={rollback} disabled={busy || running}><Undo2 size={14} /> Roll back</button>{/if}
            {#if st.custom_binary}<button class="btn sm danger" onclick={reset} disabled={busy || running}><Trash2 size={14} /> Reset to image</button>{/if}
          </div>
        {/if}
      </div>
    </div>

    <div class="card stack">
      <div class="card-head">
        <div>
          <h2>Updates</h2>
          <div class="desc">
            {#if upd?.last_check}Last checked {when(upd.last_check)}{#if upd.next_check} · next {when(upd.next_check)}{/if}{:else}Not checked yet{/if}
            {#if upd?.last_error}· <span style="color: var(--warn-soft-text)">{upd.last_error}</span>{/if}
          </div>
        </div>
        {#if can.admin()}<button class="btn ghost sm right" onclick={checkNow} disabled={busy}><RefreshCw size={14} /> Check now</button>{/if}
      </div>
      {#if upd?.last_auto}
        {@const a = upd.last_auto}
        {#if a.ok}
          <div class="alert ok"><CircleCheck /><span>Automatic update {a.from ? `from ${a.from} ` : ''}to {a.to} installed {when(a.at)}</span></div>
        {:else}
          <ErrorBox title={`Automatic update ${a.from ? `from ${a.from} ` : ''}to ${a.to} failed ${when(a.at)}`} message={a.error} />
        {/if}
      {/if}
      {#if us}
        <fieldset class="stack" disabled={!can.admin()}>
          <label class="check"><input type="checkbox" bind:checked={us.updates.check} /> Check for new Caddy releases in the background</label>
          {#if us.updates.check}
            <div class="grid2">
              <label class="field"><span>Check every</span>
                <select bind:value={us.updates.check_every_hours}>
                  {#each intervals as [h, l]}<option value={h}>{l}</option>{/each}
                  {#if !intervals.some(([h]) => h === us.updates.check_every_hours)}<option value={us.updates.check_every_hours}>{us.updates.check_every_hours} hours</option>{/if}
                </select></label>
            </div>
            <label class="check"><input type="checkbox" bind:checked={us.updates.auto} /> Install new releases automatically</label>
            <span class="hint">Same path as <strong>Update Caddy</strong>: the new build is downloaded with your plugins, checked, and your configuration is validated against it before it replaces the running one. Caddy restarts for a few seconds. A build that fails any check is discarded, the old one keeps running, and you are notified.</span>
            {#if us.updates.auto}
              <div class="grid2">
                <label class="field"><span>Only between</span><input type="time" bind:value={us.updates.window_start} /></label>
                <label class="field"><span>and</span><input type="time" bind:value={us.updates.window_end} /></label>
              </div>
              <span class="hint">Server local time (set <code>TZ</code> on the container to change it). Leave both empty to install as soon as a release is found.{#if upd && !upd.in_window} Outside the window right now.{/if}</span>
              {#if upd?.update_available && upd?.auto_blocked}<div class="muted small">Not installing yet: {upd.auto_blocked}.</div>{/if}
            {/if}
          {/if}
          {#if can.admin()}<div class="row"><button class="btn primary sm" onclick={saveUpdates} disabled={busy}>Save</button></div>{/if}
        </fieldset>
      {/if}
    </div>

    {#if st.job}
      {@const j = st.job}
      <div class="card stack job" class:running={j.status === 'running'}>
        <div class="card-head">
          <span class="s-ic" class:spin={j.status === 'running'}>
            {#if j.status === 'running'}<RefreshCw size={18} />{:else if j.status === 'done'}<CircleCheck size={18} />{:else}<CircleX size={18} />{/if}
          </span>
          <div class="grow">
            <h2>{j.kind === 'install' ? 'Installing plugins' : j.kind === 'upgrade' ? 'Updating Caddy' : j.kind === 'reset' ? 'Resetting to the image build' : 'Rolling back'}{#if j.status === 'done'}: done{:else if j.status === 'failed'}: failed{/if}</h2>
            <div class="desc">Started {when(j.started)}{#if j.packages?.length} · {j.packages.length} plugin(s){/if}</div>
          </div>
          <button class="btn ghost sm" onclick={() => (showLog = !showLog)}><ChevronDown size={15} /> {showLog ? 'Hide' : 'Show'} log</button>
        </div>
        {#if j.status === 'failed'}<ErrorBox message={j.error} />{/if}
        {#if showLog}<pre class="log">{j.log.join('\n')}</pre>{/if}
      </div>
    {/if}

    <div class="card stack">
      <div class="card-head"><div><h2>Installed</h2><div class="desc">Plugins compiled into the running Caddy</div></div></div>
      {#if st.installed.length === 0}
        <p class="muted">Only standard modules. Pick plugins below to build a custom Caddy.</p>
      {:else}
        <div class="table-wrap"><table class="list">
          <thead><tr><th>Plugin</th><th class="hide-sm">Package</th><th>Version</th><th></th></tr></thead>
          <tbody>
            {#each st.installed as i}
              <tr class:off={remove.includes(i.path)}>
                <td><strong>{i.name}</strong><div class="muted xs truncate">{i.modules.join(', ')}</div></td>
                <td class="hide-sm mono small">{i.path}</td>
                <td class="mono small">{i.version}</td>
                <td class="nowrap" style="text-align:right">
                  {#if can.admin()}
                    <button class="btn xs" class:danger={!remove.includes(i.path)} onclick={() => toggleRemove(i.path)} disabled={running || !st.supported}>
                      {#if remove.includes(i.path)}<Undo2 size={13} /> Keep{:else}<Minus size={13} /> Remove{/if}
                    </button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table></div>
      {/if}
    </div>

    <div class="card stack">
      <div class="card-head">
        <div><h2>Catalog</h2><div class="desc">{st.catalog.length} packages{#if st.catalog_fetched_at} · updated {when(st.catalog_fetched_at)}{/if}</div></div>
        <button class="btn ghost sm right" onclick={refreshCatalog} disabled={busy}><RefreshCw size={14} /> Refresh catalog</button>
      </div>
      <div class="toolbar">
        <div class="searchbox grow"><Search size={16} /><input type="search" placeholder="Search packages, modules…" bind:value={q} /></div>
        <div class="chips">
          {#each categories as [k, l]}<button class="chip" class:active={cat === k} onclick={() => (cat = k)}>{l}</button>{/each}
        </div>
      </div>
      {#if shown.length === 0}
        <p class="muted">Nothing matches.</p>
      {:else}
        <div class="plist">
          {#each shown.slice(0, 120) as p (p.path)}
            {@const inst = installedPaths.has(p.path)}
            {@const queued = add.includes(p.path)}
            <div class="prow" class:queued>
              <div class="grow" style="min-width:0">
                <div class="row tight"><strong>{p.name}</strong>{#if inst}<span class="badge ok"><CircleCheck /> installed</span>{/if}{#if p.featured}<span class="badge accent">featured</span>{/if}</div>
                <div class="muted small">{p.description}</div>
                <div class="mono xs faint truncate">{p.path}{#if p.downloads} · {fmtDl(p.downloads)} downloads{/if}</div>
              </div>
              {#if p.repo}<a class="btn ghost icon xs" href={p.repo} target="_blank" rel="noreferrer" title="Repository"><ExternalLink size={14} /></a>{/if}
              {#if can.admin() && !inst}
                <button class="btn xs" class:primary={!queued} onclick={() => toggleAdd(p.path)} disabled={running || !st.supported}>
                  {#if queued}<Undo2 size={13} /> Undo{:else}<Plus size={13} /> Add{/if}
                </button>
              {/if}
            </div>
          {/each}
          {#if shown.length > 120}<p class="muted small">Showing 120 of {shown.length}; narrow the search.</p>{/if}
        </div>
      {/if}
    </div>
  </div>

  {#if pending && can.admin()}
    <div class="actionbar">
      <div class="grow small">
        <Info size={15} />
        {#if add.length}<span>Add: <span class="mono">{add.map((p) => p.split('/').pop()).join(', ')}</span></span>{/if}
        {#if remove.length}<span> Remove: <span class="mono">{remove.map((p) => p.split('/').pop()).join(', ')}</span></span>{/if}
      </div>
      <button class="btn" onclick={() => { add = []; remove = [] }}>Discard</button>
      <button class="btn primary" onclick={applyChanges} disabled={busy || running}><Download size={15} /> Build & restart Caddy</button>
    </div>
  {/if}
{/if}

<style>
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  .status { display: flex; align-items: center; gap: 28px; flex-wrap: wrap; padding: 16px 18px; }
  .s-item { display: flex; align-items: center; gap: 12px; }
  .s-ic { width: 36px; height: 36px; border-radius: 10px; display: grid; place-items: center; background: var(--accent-soft); color: var(--accent-soft-text); flex: none; }
  .s-label { font-size: 12px; color: var(--muted); }
  .s-val { font-weight: 650; font-size: 15px; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .s-actions { margin-left: auto; display: flex; gap: 8px; flex-wrap: wrap; }
  .job.running { border-color: var(--accent); }
  .spin :global(svg) { animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .log { max-height: 320px; overflow: auto; font-size: 12.5px; }
  .toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .searchbox { position: relative; min-width: 200px; }
  .searchbox :global(svg) { position: absolute; left: 11px; top: 50%; transform: translateY(-50%); color: var(--faint); pointer-events: none; }
  .searchbox input { padding-left: 34px; }
  .chips { display: flex; gap: 6px; flex-wrap: wrap; }
  .chip { font: inherit; font-size: 12.5px; padding: 5px 10px; border-radius: 999px; border: 1px solid var(--border); background: var(--surface); color: var(--text-2); cursor: pointer; }
  .chip.active { background: var(--text); color: var(--bg); border-color: var(--text); }
  .plist { display: flex; flex-direction: column; gap: 6px; }
  .prow { display: flex; align-items: center; gap: 10px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 10px; background: var(--surface); }
  .prow.queued { border-color: var(--accent); background: var(--accent-soft); }
  tr.off td { opacity: 0.5; text-decoration: line-through; }
  .actionbar {
    position: sticky; bottom: 16px; z-index: 5; display: flex; gap: 8px; align-items: center; margin-top: 18px;
    padding: 10px 10px 10px 16px; background: color-mix(in srgb, var(--surface) 88%, transparent); backdrop-filter: blur(10px);
    border: 1px solid var(--border); border-radius: 14px; box-shadow: var(--shadow-lg);
  }
  .actionbar .grow { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  @media (max-width: 640px) { .s-actions { margin-left: 0; width: 100%; } .status { gap: 16px; } }
</style>
