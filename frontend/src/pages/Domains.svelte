<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { Plus, ShieldCheck, KeyRound, TriangleAlert, Info, Pencil, Trash2 } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import { api } from '../lib/api.js'
  import { app, toast, toastErr, refreshStatus, loadMeta } from '../lib/state.svelte.js'
  import Modal from '../lib/Modal.svelte'

  let domains = $state([])
  let creds = $state([])
  let editDom = $state(null)
  let editCred = $state(null)
  let error = $state('')

  async function load() {
    try {
      ;[domains, creds] = await Promise.all([api.get('/domains'), api.get('/dns-credentials')])
    } catch (e) { toastErr(e) }
  }
  onMount(() => { loadMeta().catch(toastErr); load() })

  const providers = $derived(app.meta?.dns_providers || [])
  const prov = (id) => providers.find((p) => p.id === id)
  const credName = (id) => creds.find((c) => c.id === id)?.name || '?'
  const challengeLabel = { 'http-01': 'HTTP', 'tls-alpn-01': 'TLS-ALPN', 'dns-01': 'DNS' }

  function newDomain() { error = ''; editDom = { name: '', challenge: 'http-01', credential_id: null, wildcard: false } }
  async function saveDomain() {
    error = ''
    try {
      const d = { ...editDom, credential_id: editDom.challenge === 'dns-01' ? Number(editDom.credential_id) || null : null }
      if (d.id) await api.put('/domains/' + d.id, d); else await api.post('/domains', d)
      editDom = null; toast('Domain saved. Apply to use it.'); load(); refreshStatus()
    } catch (e) { error = e.message }
  }
  async function delDomain(d) {
    if (!confirm(`Remove ${d.name}? Sites under it go back to default certificate handling.`)) return
    try { await api.del('/domains/' + d.id); load(); refreshStatus() } catch (e) { toastErr(e) }
  }

  function newCred() { error = ''; editCred = { name: '', provider: providers.find((p) => p.available)?.id || providers[0]?.id || '', fields: {} } }
  function openCred(c) { error = ''; editCred = { id: c.id, name: c.name, provider: c.provider, fields: { ...c.fields }, set: c.set } }
  async function saveCred() {
    error = ''
    try {
      const body = { name: editCred.name, provider: editCred.provider, fields: editCred.fields }
      if (editCred.id) await api.put('/dns-credentials/' + editCred.id, body); else await api.post('/dns-credentials', body)
      editCred = null; toast('DNS account saved'); load(); refreshStatus()
    } catch (e) { error = e.message }
  }
  async function delCred(c) {
    if (!confirm(`Delete DNS account ${c.name}?`)) return
    try { await api.del('/dns-credentials/' + c.id); load() } catch (e) { toastErr(e) }
  }
  const cp = $derived(editCred ? prov(editCred.provider) : null)
</script>

<div class="page-head">
  <div>
    <h1>Domains & DNS</h1>
    <div class="sub">Optional. By default Caddy gets a certificate for each hostname over HTTP. Add a domain here to use DNS validation (works behind firewalls, needed for wildcard certificates).</div>
  </div>
</div>

<div class="stack">
  <div class="card stack">
    <div class="card-head"><span class="h-ic"><ShieldCheck size={18} /></span><div><h2>Domains</h2><div class="desc">Certificate validation per domain</div></div><button class="btn sm right" onclick={newDomain}><Plus size={15} /> Add domain</button></div>
    {#if domains.length === 0}
      <p class="muted">No domains configured. Every site uses Caddy's automatic HTTPS.</p>
    {:else}
      <div class="table-wrap"><table class="list">
        <thead><tr><th>Domain</th><th>Validation</th><th class="hide-sm">DNS account</th><th></th></tr></thead>
        <tbody>
          {#each domains as d}
            <tr>
              <td><strong>{d.name}</strong> {#if d.wildcard}<span class="badge accent">*.{d.name}</span>{/if}</td>
              <td>{challengeLabel[d.challenge] || d.challenge}</td>
              <td class="hide-sm">{d.credential_id ? credName(d.credential_id) : '—'}</td>
              <td class="nowrap right-cell">
                <button class="btn xs" onclick={() => { error = ''; editDom = { ...d } }}><Pencil size={13} /> Edit</button>
                <button class="btn ghost icon xs danger" aria-label="Remove" onclick={() => delDomain(d)}><Trash2 size={14} /></button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table></div>
    {/if}
  </div>

  <div class="card stack">
    <div class="card-head"><span class="h-ic"><KeyRound size={18} /></span><div><h2>DNS provider accounts</h2><div class="desc">API tokens, encrypted at rest</div></div><button class="btn sm right" onclick={newCred}><Plus size={15} /> Add account</button></div>
    {#if creds.length === 0}
      <p class="muted">API credentials for your DNS provider, used for DNS validation. Stored encrypted.</p>
    {:else}
      <div class="table-wrap"><table class="list">
        <thead><tr><th>Name</th><th>Provider</th><th></th></tr></thead>
        <tbody>
          {#each creds as c}
            <tr>
              <td><strong>{c.name}</strong></td>
              <td>{prov(c.provider)?.name || c.provider} {#if prov(c.provider) && !prov(c.provider).available}<span class="badge warn">plugin missing</span>{/if}</td>
              <td class="nowrap right-cell">
                <button class="btn xs" onclick={() => openCred(c)}><Pencil size={13} /> Edit</button>
                <button class="btn ghost icon xs danger" aria-label="Delete" onclick={() => delCred(c)}><Trash2 size={14} /></button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table></div>
    {/if}
  </div>
</div>

{#if editDom}
  <Modal title={editDom.id ? 'Edit domain' : 'Add domain'} onclose={() => (editDom = null)}>
    <label class="field"><span>Domain</span><input type="text" bind:value={editDom.name} placeholder="example.com" />
      <span class="hint">Applies to this domain and all its subdomains.</span></label>
    <label class="field"><span>How to prove you own it</span>
      <select bind:value={editDom.challenge}>
        <option value="http-01">HTTP (default; port 80 must be reachable from the internet)</option>
        <option value="tls-alpn-01">TLS-ALPN (port 443 must be reachable)</option>
        <option value="dns-01">DNS (uses your DNS provider's API; works without open ports)</option>
      </select></label>
    {#if editDom.challenge === 'dns-01'}
      <label class="field"><span>DNS account</span>
        <select bind:value={editDom.credential_id}>
          <option value={null}>Choose…</option>
          {#each creds as c}<option value={c.id}>{c.name}</option>{/each}
        </select>
        {#if creds.length === 0}<span class="hint">Add a DNS provider account first.</span>{/if}
      </label>
      <label class="check"><input type="checkbox" bind:checked={editDom.wildcard} /> Use one wildcard certificate (*.{editDom.name || 'domain'}) for all subdomains</label>
    {/if}
    {#if error}<ErrorBox message={error} />{/if}
    {#snippet footer()}
      <button class="btn" onclick={() => (editDom = null)}>Cancel</button>
      <button class="btn primary" onclick={saveDomain}>Save</button>
    {/snippet}
  </Modal>
{/if}

{#if editCred}
  <Modal title={editCred.id ? 'Edit DNS account' : 'Add DNS account'} onclose={() => (editCred = null)}>
    <label class="field"><span>Provider</span>
      <select bind:value={editCred.provider} disabled={!!editCred.id}>
        {#each providers as p}<option value={p.id}>{p.name}{p.available ? '' : ' (plugin not installed)'}</option>{/each}
      </select></label>
    {#if cp}
      {#if !cp.available}
        <div class="alert warn"><TriangleAlert /><span>Your Caddy build doesn't include the <code>{cp.plugin}</code> plugin. Add it to your Caddy image (see README) before using this provider.</span></div>
      {/if}
      <div class="alert info small"><Info /><span>{cp.privilege} {#if cp.docs_url}<a href={cp.docs_url} target="_blank" rel="noreferrer">How to create a token</a>{/if}</span></div>
      <label class="field"><span>Name</span><input type="text" bind:value={editCred.name} placeholder={cp.name} /></label>
      {#each cp.fields as f}
        <label class="field"><span>{f.label}{f.required || /optional/i.test(f.label) ? '' : ' (optional)'}</span>
          <input type={f.secret ? 'password' : 'text'} bind:value={editCred.fields[f.key]} autocomplete="off"
            placeholder={f.secret && editCred.set?.[f.key] ? '•••••• (saved; leave empty to keep)' : f.default || ''} />
          {#if f.help}<span class="hint">{f.help}</span>{/if}
        </label>
      {/each}
      <p class="muted small">Tip: instead of the value you can enter <code>env:VAR_NAME</code> or <code>file:/run/secrets/name</code> to read it from the Caddy container.</p>
    {/if}
    {#if error}<ErrorBox message={error} />{/if}
    {#snippet footer()}
      <button class="btn" onclick={() => (editCred = null)}>Cancel</button>
      <button class="btn primary" onclick={saveCred}>Save</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .right-cell { text-align: right; }
  .h-ic { width: 36px; height: 36px; border-radius: 10px; display: grid; place-items: center; background: var(--accent-soft); color: var(--accent-soft-text); flex: none; }
</style>
