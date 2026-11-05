<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { Plus, KeyRound } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import { api } from '../lib/api.js'
  import { app, toast, toastErr } from '../lib/state.svelte.js'
  import { ago, when } from '../lib/util.js'
  import Modal from '../lib/Modal.svelte'

  let tab = $state('users')
  let users = $state([])
  let keys = $state([])
  let edit = $state(null)
  let newKey = $state(null)
  let created = $state(null)
  let error = $state('')

  async function load() {
    try { ;[users, keys] = await Promise.all([api.get('/users'), api.get('/api-keys')]) } catch (e) { toastErr(e) }
  }
  onMount(load)

  async function saveUser() {
    error = ''
    try {
      const body = { username: edit.username, role: edit.role, disabled: edit.disabled, password: edit.password || '', reset_2fa: !!edit.reset_2fa }
      if (edit.id) await api.put('/users/' + edit.id, body); else await api.post('/users', body)
      edit = null; toast('User saved'); load()
    } catch (e) { error = e.message }
  }
  async function delUser() {
    if (!confirm(`Delete ${edit.username}?`)) return
    try { await api.del('/users/' + edit.id); edit = null; load() } catch (e) { error = e.message }
  }
  async function createKey() {
    error = ''
    try { created = await api.post('/api-keys', { ...newKey, valid_days: Number(newKey.valid_days) || 0 }); newKey = null; load() } catch (e) { error = e.message }
  }
  async function delKey(k) {
    if (!confirm(`Revoke key ${k.name}? Scripts using it stop working.`)) return
    try { await api.del('/api-keys/' + k.id); load() } catch (e) { toastErr(e) }
  }
  const roleHelp = { admin: 'everything, including users, domains and settings', editor: 'edit sites and apply changes', viewer: 'read-only', member: 'sign in to your protected sites only, no panel' }
</script>

<div class="page-head">
  <div><h1>Users & API keys</h1><div class="sub">Who can use Caddymium, and keys for scripts.</div></div>
</div>

<div class="tabs">
  <button class:active={tab === 'users'} onclick={() => (tab = 'users')}>Users</button>
  <button class:active={tab === 'keys'} onclick={() => (tab = 'keys')}>API keys</button>
</div>

{#if tab === 'users'}
  <div class="stack">
    <div><button class="btn primary" onclick={() => { error = ''; edit = { username: '', role: 'editor', password: '', disabled: false } }}><Plus size={16} /> Add user</button></div>
    <div class="card flush table-wrap">
      <table class="list">
        <thead><tr><th>User</th><th>Role</th><th class="hide-sm">2FA</th><th class="hide-sm">Last login</th></tr></thead>
        <tbody>
          {#each users as u}
            <tr class="clickable" onclick={() => { error = ''; edit = { ...u, password: '' } }}>
              <td><strong>{u.username}</strong> {#if u.id === app.me.user.id}<span class="badge">you</span>{/if} {#if u.disabled}<span class="badge err">disabled</span>{/if}</td>
              <td>{u.role}</td>
              <td class="hide-sm">{u.totp_enabled ? 'on' : '—'}</td>
              <td class="hide-sm" title={when(u.last_login)}>{u.last_login ? ago(u.last_login) : 'never'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
{:else}
  <div class="stack">
    <p class="muted">Use a key with <code>Authorization: Bearer &lt;key&gt;</code> against <code>/api/…</code>, e.g. <code>curl -H "Authorization: Bearer chk_…" https://panel/api/sites</code>. A key never has more rights than its owner.</p>
    <div><button class="btn primary" onclick={() => { error = ''; newKey = { name: '', role: 'viewer', valid_days: 0 } }}><KeyRound size={16} /> Create key</button></div>
    {#if keys.length}
      <div class="card flush table-wrap">
        <table class="list">
          <thead><tr><th>Name</th><th>Key</th><th>Role</th><th class="hide-sm">Last used</th><th class="hide-sm">Expires</th><th></th></tr></thead>
          <tbody>
            {#each keys as k}
              <tr>
                <td>{k.name || '—'}</td><td class="mono small">{k.prefix}…</td><td>{k.role}</td>
                <td class="hide-sm">{k.last_used ? ago(k.last_used) : 'never'}</td>
                <td class="hide-sm">{k.expires_at ? when(k.expires_at) : 'never'}</td>
                <td><button class="btn sm danger" onclick={() => delKey(k)}>Revoke</button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
{/if}

{#if edit}
  <Modal title={edit.id ? 'Edit user' : 'Add user'} onclose={() => (edit = null)}>
    <label class="field"><span>Username</span><input type="text" bind:value={edit.username} /></label>
    <label class="field"><span>Role</span>
      <select bind:value={edit.role}>{#each ['admin', 'editor', 'viewer', 'member'] as r}<option value={r}>{r}: {roleHelp[r]}</option>{/each}</select></label>
    <label class="field"><span>{edit.id ? 'New password (leave empty to keep)' : 'Password'}</span><input type="password" bind:value={edit.password} autocomplete="new-password" /></label>
    {#if edit.id}
      <label class="check"><input type="checkbox" bind:checked={edit.disabled} /> Disabled (can't log in)</label>
      {#if edit.totp_enabled}<label class="check"><input type="checkbox" bind:checked={edit.reset_2fa} /> Reset two-factor (lost phone)</label>{/if}
    {/if}
    {#if error}<ErrorBox message={error} />{/if}
    {#snippet footer()}
      {#if edit.id && edit.id !== app.me.user.id}<button class="btn danger" style="margin-right:auto" onclick={delUser}>Delete</button>{/if}
      <button class="btn" onclick={() => (edit = null)}>Cancel</button>
      <button class="btn primary" onclick={saveUser}>Save</button>
    {/snippet}
  </Modal>
{/if}

{#if newKey}
  <Modal title="Create API key" onclose={() => (newKey = null)}>
    <label class="field"><span>Name</span><input type="text" bind:value={newKey.name} placeholder="backup script" /></label>
    <label class="field"><span>Role</span><select bind:value={newKey.role}>{#each ['viewer', 'editor', 'admin'] as r}<option value={r}>{r}</option>{/each}</select></label>
    <label class="field"><span>Expires after (days, 0 = never)</span><input type="number" bind:value={newKey.valid_days} /></label>
    {#if error}<ErrorBox message={error} />{/if}
    {#snippet footer()}
      <button class="btn" onclick={() => (newKey = null)}>Cancel</button>
      <button class="btn primary" onclick={createKey}>Create</button>
    {/snippet}
  </Modal>
{/if}

{#if created}
  <Modal title="Your new API key" onclose={() => (created = null)}>
    <div class="alert warn">Copy it now. It won't be shown again.</div>
    <pre style="white-space:pre-wrap;word-break:break-all">{created.token}</pre>
    {#snippet footer()}
      <button class="btn" onclick={() => { navigator.clipboard.writeText(created.token); toast('Copied') }}>Copy</button>
      <button class="btn primary" onclick={() => (created = null)}>Done</button>
    {/snippet}
  </Modal>
{/if}
