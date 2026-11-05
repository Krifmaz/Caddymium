<script>
  import { Plus, Bell, Send } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import { api } from '../lib/api.js'
  import { app, toast, toastErr, loadMeta } from '../lib/state.svelte.js'
  import Modal from '../lib/Modal.svelte'

  let channels = $state([])
  let edit = $state(null)
  let error = $state('')
  let testing = $state(false)

  const typeNames = { webhook: 'Webhook', ntfy: 'ntfy', gotify: 'Gotify', discord: 'Discord', slack: 'Slack', email: 'Email (SMTP)' }
  const eventNames = {
    apply_failed: 'Config apply failed', cert_expiring: 'Certificate expiring', cert_error: 'Certificate error',
    upstream_down: 'Upstream down', upstream_up: 'Upstream back up', update_available: 'Caddy update available', update_installed: 'Caddy updated automatically', update_failed: 'Automatic Caddy update failed',
    backup_failed: 'Backup failed', discovery: 'New container discovered',
    caddy_down: 'Caddy is down', caddy_recovered: 'Caddy rolled back or restarted',
  }
  const fieldLabels = { url: 'URL', token: 'Token', priority: 'Priority', header: 'Extra header (Name: value)', host: 'SMTP host', port: 'Port', username: 'Username', password: 'Password', from: 'From address', to: 'To address', tls: 'TLS (starttls / tls / none)' }
  const secret = new Set(['token', 'password', 'header'])

  async function load() { try { channels = await api.get('/notifications') } catch (e) { toastErr(e) } }
  onMount(() => { loadMeta().catch(toastErr); load() })

  const types = $derived(app.meta?.notification_types || {})
  const events = $derived(app.meta?.notification_events || [])

  function open(c) {
    error = ''
    edit = c ? { ...c, config: { ...c.config }, events: [...(c.events || [])] } : { name: '', type: 'ntfy', config: {}, events: [], enabled: true }
  }
  function toggleEvent(e, on) { edit.events = on ? [...new Set([...edit.events, e])] : edit.events.filter((x) => x !== e) }
  async function save() {
    error = ''
    try {
      if (edit.id) await api.put('/notifications/' + edit.id, edit); else await api.post('/notifications', edit)
      edit = null; toast('Saved'); load()
    } catch (e) { error = e.message }
  }
  async function test() {
    error = ''; testing = true
    try { await api.post('/notifications/test', edit); toast('Test message sent') } catch (e) { error = 'Test failed: ' + e.message }
    testing = false
  }
  async function del() {
    if (!confirm(`Delete ${edit.name}?`)) return
    try { await api.del('/notifications/' + edit.id); edit = null; load() } catch (e) { toastErr(e) }
  }
</script>

<div class="page-head">
  <div><h1>Notifications</h1><div class="sub">Get told when an apply fails or something needs attention.</div></div>
  <button class="btn primary right" onclick={() => open(null)}><Plus size={16} /> Add channel</button>
</div>

{#if channels.length === 0}
  <div class="card empty"><div class="icon-wrap"><Bell size={22} /></div><p>No channels yet. ntfy, Gotify, Discord, Slack, email and plain webhooks are supported.</p></div>
{:else}
  <div class="card flush table-wrap">
    <table class="list">
      <thead><tr><th>Name</th><th>Type</th><th class="hide-sm">Events</th><th>Status</th></tr></thead>
      <tbody>
        {#each channels as c}
          <tr class="clickable" onclick={() => open(c)}>
            <td><strong>{c.name}</strong></td><td>{typeNames[c.type] || c.type}</td>
            <td class="hide-sm small">{c.events?.length ? c.events.map((e) => eventNames[e] || e).join(', ') : 'all events'}</td>
            <td>{#if c.enabled}<span class="badge ok">on</span>{:else}<span class="badge">off</span>{/if}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

{#if edit}
  <Modal title={edit.id ? 'Edit channel' : 'Add channel'} onclose={() => (edit = null)}>
    <div class="grid2">
      <label class="field"><span>Name</span><input type="text" bind:value={edit.name} placeholder="My phone" /></label>
      <label class="field"><span>Type</span>
        <select bind:value={edit.type}>{#each Object.keys(types) as t}<option value={t}>{typeNames[t] || t}</option>{/each}</select></label>
    </div>
    {#each types[edit.type] || [] as f}
      <label class="field"><span>{fieldLabels[f] || f}</span>
        <input type={secret.has(f) ? 'password' : 'text'} bind:value={edit.config[f]} autocomplete="off" /></label>
    {/each}
    <div class="field"><span>Events (none selected = all)</span>
      <div class="evs">
        {#each events as e}
          <label class="check"><input type="checkbox" checked={edit.events.includes(e)} onchange={(x) => toggleEvent(e, x.target.checked)} /> {eventNames[e] || e}</label>
        {/each}
      </div>
    </div>
    <label class="check"><input type="checkbox" bind:checked={edit.enabled} /> Enabled</label>
    {#if error}<div class="alert err">{error}</div>{/if}
    {#snippet footer()}
      {#if edit.id}<button class="btn danger" style="margin-right:auto" onclick={del}>Delete</button>{/if}
      <button class="btn" onclick={test} disabled={testing}><Send size={15} /> {testing ? 'Sending…' : 'Send test'}</button>
      <button class="btn primary" onclick={save}>Save</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .evs { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 6px; }
</style>
