<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { Plus, Braces, FileCode, Check, Pencil } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import { api } from '../lib/api.js'
  import { toast, toastErr, refreshStatus, can } from '../lib/state.svelte.js'
  import Modal from '../lib/Modal.svelte'
  import Switch from '../lib/Switch.svelte'

  let tab = $state('snippets')
  let snippets = $state([])
  let blocks = $state([])
  let edit = $state(null) // { type: 'snippet'|'block', ... }
  let error = $state('')
  let check = $state(null)

  async function load() {
    try { ;[snippets, blocks] = await Promise.all([api.get('/snippets'), api.get('/raw-blocks')]) } catch (e) { toastErr(e) }
  }
  onMount(load)

  function open(type, item) {
    error = ''; check = null
    edit = item ? { type, ...item } : type === 'snippet' ? { type, name: '', description: '', body: '' } : { type, name: '', body: '', enabled: true }
  }
  async function validate() {
    check = null
    try { check = await api.post('/config/validate', { text: edit.body, context: edit.type === 'snippet' ? 'site' : 'top' }) } catch (e) { check = { valid: false, error: e.message } }
  }
  async function save() {
    error = ''
    const { type, ...body } = edit
    const base = type === 'snippet' ? '/snippets' : '/raw-blocks'
    try {
      if (body.id) await api.put(`${base}/${body.id}`, body); else await api.post(base, body)
      edit = null; toast('Saved. Apply to make it live.'); load(); refreshStatus()
    } catch (e) { error = e.message }
  }
  async function del() {
    if (!confirm(`Delete ${edit.name || 'this block'}?`)) return
    const base = edit.type === 'snippet' ? '/snippets' : '/raw-blocks'
    try { await api.del(`${base}/${edit.id}`); edit = null; load(); refreshStatus() } catch (e) { toastErr(e) }
  }
  async function toggleBlock(b, on) {
    try { await api.put('/raw-blocks/' + b.id, { ...b, enabled: on }); b.enabled = on; refreshStatus() } catch (e) { toastErr(e) }
  }
  const snippetPh = 'header {\n\tStrict-Transport-Security "max-age=31536000"\n\tX-Frame-Options DENY\n}'
  const blockPh = 'old.example.com {\n\treverse_proxy 10.0.0.9:80\n}'
</script>

<div class="page-head">
  <div>
    <h1>Snippets & raw blocks</h1>
    <div class="sub">For anything the forms don't cover. Snippets are reusable pieces you attach to sites; raw blocks are copied into the Caddyfile as-is.</div>
  </div>
</div>

<div class="tabs">
  <button class:active={tab === 'snippets'} onclick={() => (tab = 'snippets')}>Snippets ({snippets.length})</button>
  <button class:active={tab === 'blocks'} onclick={() => (tab = 'blocks')}>Raw blocks ({blocks.length})</button>
</div>

{#if tab === 'snippets'}
  <div class="stack">
    {#if can.edit()}<div><button class="btn primary" onclick={() => open('snippet')}><Plus size={16} /> New snippet</button></div>{/if}
    {#if snippets.length === 0}
      <div class="card empty"><div class="icon-wrap"><Braces size={22} /></div><p>Example: a “security-headers” snippet with HSTS and X-Frame-Options that you tick on every site.</p></div>
    {/if}
    {#each snippets as sn}
      <button class="card item" onclick={() => open('snippet', sn)}>
        <div class="row"><strong class="mono">({sn.name})</strong><span class="muted small">{sn.description}</span></div>
        <pre class="peek">{sn.body}</pre>
      </button>
    {/each}
  </div>
{:else}
  <div class="stack">
    {#if can.admin()}<div><button class="btn primary" onclick={() => open('block')}><Plus size={16} /> New raw block</button></div>{/if}
    {#if blocks.length === 0}
      <div class="card empty"><div class="icon-wrap"><FileCode size={22} /></div><p>Raw blocks hold top-level Caddyfile text, e.g. sites using features Caddymium doesn't model. Imported blocks it couldn't understand also land here, untouched.</p></div>
    {/if}
    {#each blocks as b}
      <div class="card stack">
        <div class="row">
          <strong>{b.name || 'Untitled block'}</strong>
          {#if b.source === 'import'}<span class="badge">imported</span>{/if}
          <span class="right row">
            {#if can.admin()}
              <Switch checked={b.enabled} label="Enabled" onchange={(v) => toggleBlock(b, v)} />
              <button class="btn xs" onclick={() => open('block', b)}><Pencil size={13} /> Edit</button>
            {/if}
          </span>
        </div>
        <pre class="peek">{b.body}</pre>
      </div>
    {/each}
  </div>
{/if}

{#if edit}
  <Modal title={edit.type === 'snippet' ? (edit.id ? 'Edit snippet' : 'New snippet') : edit.id ? 'Edit raw block' : 'New raw block'} wide onclose={() => (edit = null)}>
    <div class="grid2">
      <label class="field"><span>Name</span><input type="text" bind:value={edit.name} placeholder={edit.type === 'snippet' ? 'security-headers' : 'legacy sites'} /></label>
      {#if edit.type === 'snippet'}
        <label class="field"><span>Description</span><input type="text" bind:value={edit.description} /></label>
      {/if}
    </div>
    <label class="field">
      <span>{edit.type === 'snippet' ? 'Directives' : 'Caddyfile text'}</span>
      <textarea rows="14" bind:value={edit.body} placeholder={edit.type === 'snippet' ? snippetPh : blockPh}></textarea>
    </label>
    <div class="row">
      <button class="btn sm" onclick={validate}><Check size={15} /> Check syntax</button>
      {#if check}{#if check.valid}<span class="badge ok">looks valid</span>{:else}<span class="mono small" style="color:var(--danger)">{check.error}</span>{/if}{/if}
    </div>
    {#if error}<ErrorBox message={error} />{/if}
    {#snippet footer()}
      {#if edit.id}<button class="btn danger" style="margin-right:auto" onclick={del}>Delete</button>{/if}
      <button class="btn" onclick={() => (edit = null)}>Cancel</button>
      <button class="btn primary" onclick={save}>Save</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .item { text-align: left; font: inherit; color: inherit; cursor: pointer; display: flex; flex-direction: column; gap: 8px; }
  .item:hover { border-color: var(--accent); }
  .peek { max-height: 140px; overflow: hidden; }
</style>
