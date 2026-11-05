<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { onMount } from 'svelte'
  import { api } from '../lib/api.js'
  import { app, toastErr, refreshStatus } from '../lib/state.svelte.js'
  import { go } from '../lib/router.svelte.js'
  import Import from './Import.svelte'

  let step = $state(1)
  let settings = $state(null)
  let imported = $state(null)
  let error = $state('')

  onMount(async () => { try { settings = await api.get('/settings') } catch (e) { toastErr(e) } })

  async function saveBasics() {
    error = ''
    try { settings = await api.put('/settings', settings); step = 2 } catch (e) { error = e.message }
  }
  async function finish(where) {
    try {
      settings = await api.put('/settings', { ...settings, setup_complete: true })
      app.me = { ...app.me, setup_complete: true }
      refreshStatus()
      go(where)
    } catch (e) { toastErr(e) }
  }
</script>

<div class="wrap stack">
  <div>
    <h1>Let's set up Caddymium</h1>
    <p class="muted">Two quick steps. Everything can be changed later in Settings.</p>
  </div>
  <div class="steps row">
    <span class:on={step === 1}>1. Certificates</span><span class="muted">→</span>
    <span class:on={step === 2}>2. Existing config</span><span class="muted">→</span>
    <span class:on={step === 3}>3. Done</span>
  </div>

  {#if !settings}
    <p class="muted">Loading…</p>
  {:else if step === 1}
    <div class="card stack">
      <h2>HTTPS certificates</h2>
      <p class="muted">Caddy gets free certificates for your sites automatically. The certificate authority emails you only if something needs attention.</p>
      <label class="field"><span>Email for certificate notices</span><input type="email" bind:value={settings.acme_email} placeholder="you@example.com" /></label>
      <label class="field"><span>Certificate authority</span>
        <select bind:value={settings.acme_ca}>
          <option value="letsencrypt">Let's Encrypt (recommended)</option>
          <option value="letsencrypt_staging">Let's Encrypt staging (testing: untrusted certs, no rate limits)</option>
          <option value="zerossl">ZeroSSL</option>
          <option value="google">Google Trust Services</option>
        </select></label>
      {#if error}<ErrorBox message={error} />{/if}
      <div class="row"><button class="btn primary" onclick={saveBasics}>Continue</button></div>
    </div>
  {:else if step === 2}
    <div class="card stack">
      <h2>Bring in your existing config</h2>
      <p class="muted">Caddymium looks for the config your Caddy is already running (or ran before). You can also paste a Caddyfile. Anything it can't turn into forms is kept for you to review.</p>
    </div>
    {#if imported}
      <div class="alert ok">Imported {imported.sites} site(s). You'll review the result before anything goes live.</div>
    {:else}
      <Import embedded ondone={(d) => (imported = d)} />
    {/if}
    <div class="row">
      <button class="btn primary" onclick={() => (step = 3)}>{imported ? 'Continue' : 'Skip, start fresh'}</button>
    </div>
  {:else}
    <div class="card stack">
      <h2>You're ready</h2>
      <p>How Caddymium works:</p>
      <ol>
        <li><strong>Edit</strong> sites with forms (or raw Caddyfile text when you need to).</li>
        <li><strong>Review</strong>: Caddymium generates the Caddyfile, Caddy validates it, and you see a diff.</li>
        <li><strong>Apply</strong>: it goes live. If Caddy isn't healthy afterwards, the previous config is restored automatically.</li>
      </ol>
      <div class="row">
        {#if imported}
          <button class="btn primary" onclick={() => finish('/changes')}>Review the imported config</button>
        {:else}
          <button class="btn primary" onclick={() => finish('/sites/new')}>Add my first site</button>
        {/if}
        <button class="btn" onclick={() => finish('/sites')}>Go to dashboard</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .wrap { max-width: 760px; }
  .steps { gap: 8px; font-size: 13px; color: var(--muted); }
  .steps .on { color: var(--accent); font-weight: 600; }
  ol { margin: 0; padding-left: 20px; display: flex; flex-direction: column; gap: 6px; }
</style>
