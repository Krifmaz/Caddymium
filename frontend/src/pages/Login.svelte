<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import { CircleAlert, ShieldCheck, Rocket, Undo2, GitCompareArrows } from '@lucide/svelte'
  import { api, setCSRF } from '../lib/api.js'
  import { app } from '../lib/state.svelte.js'

  let { onlogin } = $props()
  let username = $state('')
  let password = $state('')
  let password2 = $state('')
  let token = $state('')
  let code = $state('')
  let error = $state('')
  let busy = $state(false)

  const setup = $derived(app.me?.needs_setup)
  const twofa = $derived(app.me?.pending_2fa)

  async function submit(e) {
    e.preventDefault()
    error = ''
    if (setup && password !== password2) { error = "Passwords don't match"; return }
    busy = true
    try {
      let res
      if (twofa) res = await api.post('/login/totp', { code })
      else if (setup) res = await api.post('/setup', { username, password, token })
      else res = await api.post('/login', { username, password })
      setCSRF(res.csrf)
      if (res.pending_2fa) { app.me = { ...app.me, ...res }; busy = false; return }
      await onlogin()
    } catch (err) {
      error = err.message
    }
    busy = false
  }
</script>

<div class="wrap">
  <div class="panel">
    <form class="box" onsubmit={submit}>
      <div class="brand"><img src="/favicon.svg" alt="" width="36" height="36" /><span>Caddymium</span></div>

      {#if twofa}
        <div class="head"><h1>Two-factor code</h1><p class="muted">Enter the 6-digit code from your authenticator app.</p></div>
        <label class="field"><span>Code</span>
          <input type="text" inputmode="numeric" autocomplete="one-time-code" bind:value={code} required placeholder="123 456" class="code" />
        </label>
      {:else if setup}
        <div class="head"><h1>Create your admin account</h1><p class="muted">This account manages your Caddy server. You can add more people later.</p></div>
        {#if app.me?.setup_token_required}
          <label class="field"><span>Setup token</span>
            <input type="password" bind:value={token} required />
            <span class="hint">The value of <code>CADDYMIUM_SETUP_TOKEN</code> on your server.</span>
          </label>
        {/if}
        <label class="field"><span>Username</span><input type="text" autocomplete="username" bind:value={username} required /></label>
        <label class="field"><span>Password</span>
          <input type="password" autocomplete="new-password" bind:value={password} required minlength="10" />
          <span class="hint">At least 10 characters. A few random words work well.</span>
        </label>
        <label class="field"><span>Repeat password</span><input type="password" autocomplete="new-password" bind:value={password2} required /></label>
      {:else}
        <div class="head"><h1>Welcome back</h1><p class="muted">Log in to manage your Caddy server.</p></div>
        <label class="field"><span>Username</span><input type="text" autocomplete="username" bind:value={username} required /></label>
        <label class="field"><span>Password</span><input type="password" autocomplete="current-password" bind:value={password} required /></label>
      {/if}

      {#if error}<ErrorBox message={error} />{/if}
      <button class="btn primary big" type="submit" disabled={busy}>
        {busy ? 'Please wait…' : twofa ? 'Verify' : setup ? 'Create account' : 'Log in'}
      </button>
    </form>
  </div>

  <div class="side">
    <div class="side-inner">
      <h2>Edit Caddy without the fear.</h2>
      <ul>
        <li><span class="ic"><GitCompareArrows size={18} /></span><div><strong>See every change</strong><span>A clear diff of the Caddyfile before anything goes live.</span></div></li>
        <li><span class="ic"><ShieldCheck size={18} /></span><div><strong>Validated by Caddy</strong><span>Broken configs never reach your server.</span></div></li>
        <li><span class="ic"><Undo2 size={18} /></span><div><strong>Automatic rollback</strong><span>If Caddy isn't healthy after an apply, the last good config comes back.</span></div></li>
        <li><span class="ic"><Rocket size={18} /></span><div><strong>Yours to keep</strong><span>Export the plain Caddyfile at any time. No lock-in.</span></div></li>
      </ul>
    </div>
  </div>
</div>

<style>
  .wrap { min-height: 100vh; min-height: 100dvh; display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
  .panel { display: grid; place-items: center; padding: 32px 20px; }
  .box { width: 100%; max-width: 380px; display: flex; flex-direction: column; gap: 18px; }
  .brand { display: flex; align-items: center; gap: 10px; font-weight: 700; font-size: 18px; letter-spacing: -0.02em; margin-bottom: 10px; }
  .head { display: flex; flex-direction: column; gap: 6px; margin-bottom: 4px; }
  .big { height: 42px; font-size: 14px; margin-top: 4px; }
  .code { font-family: var(--mono); font-size: 18px; letter-spacing: 0.2em; height: 46px; text-align: center; }

  .side {
    position: relative; overflow: hidden; display: grid; place-items: center; padding: 40px; color: #ecfdf5;
    background:
      radial-gradient(600px 400px at 80% 10%, rgba(52, 211, 153, 0.35), transparent 60%),
      radial-gradient(500px 500px at 0% 100%, rgba(14, 116, 144, 0.5), transparent 60%),
      linear-gradient(160deg, #064e3b, #022c22 60%, #031d17);
  }
  .side::after {
    content: ''; position: absolute; inset: 0; opacity: 0.12; pointer-events: none;
    background-image: linear-gradient(rgba(255,255,255,.5) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,.5) 1px, transparent 1px);
    background-size: 40px 40px; mask-image: radial-gradient(ellipse at center, #000 30%, transparent 75%);
  }
  .side-inner { position: relative; z-index: 1; max-width: 420px; }
  .side h2 { font-size: 30px; line-height: 1.15; letter-spacing: -0.03em; font-weight: 700; margin-bottom: 28px; color: #fff; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 20px; }
  li { display: flex; gap: 14px; align-items: flex-start; }
  li div { display: flex; flex-direction: column; gap: 2px; }
  li strong { color: #fff; font-weight: 600; }
  li span:not(.ic) { color: rgba(236, 253, 245, 0.72); font-size: 13.5px; }
  .ic {
    width: 36px; height: 36px; border-radius: 10px; display: grid; place-items: center; flex: none;
    background: rgba(255, 255, 255, 0.1); border: 1px solid rgba(255, 255, 255, 0.15); color: #6ee7b7;
  }
  @media (max-width: 900px) {
    .wrap { grid-template-columns: 1fr; }
    .side { display: none; }
  }
</style>
