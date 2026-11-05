<script>
  import ErrorBox from '../lib/ErrorBox.svelte'
  import QRCode from 'qrcode'
  import { api } from '../lib/api.js'
  import { app, toast } from '../lib/state.svelte.js'

  let current = $state('')
  let next = $state('')
  let next2 = $state('')
  let pwErr = $state('')

  let enroll = $state(null) // { secret, uri, qr }
  let code = $state('')
  let disablePw = $state('')
  let tfErr = $state('')

  async function changePassword(e) {
    e.preventDefault()
    pwErr = ''
    if (next !== next2) { pwErr = "New passwords don't match"; return }
    try {
      await api.post('/me/password', { current, new: next })
      current = next = next2 = ''
      toast('Password changed. Other sessions were logged out.')
    } catch (err) { pwErr = err.message }
  }
  async function begin() {
    tfErr = ''
    try {
      const r = await api.post('/me/totp/begin')
      enroll = { ...r, qr: await QRCode.toDataURL(r.uri, { margin: 1, width: 200 }) }
    } catch (err) { tfErr = err.message }
  }
  async function enable() {
    tfErr = ''
    try {
      await api.post('/me/totp/enable', { code })
      app.me.user.totp_enabled = true
      enroll = null; code = ''
      toast('Two-factor authentication is on')
    } catch (err) { tfErr = err.message }
  }
  async function disable() {
    tfErr = ''
    try {
      await api.post('/me/totp/disable', { password: disablePw })
      app.me.user.totp_enabled = false
      disablePw = ''
      toast('Two-factor authentication is off')
    } catch (err) { tfErr = err.message }
  }
</script>

<div class="page-head">
  <div><h1>Your account</h1><div class="sub">{app.me.user.username} · {app.me.user.role} · Caddymium {app.me.version}</div></div>
</div>

<div class="stack" style="max-width:560px">
  <form class="card stack" onsubmit={changePassword}>
    <h2>Change password</h2>
    <label class="field"><span>Current password</span><input type="password" bind:value={current} autocomplete="current-password" required /></label>
    <label class="field"><span>New password</span><input type="password" bind:value={next} autocomplete="new-password" required minlength="10" /></label>
    <label class="field"><span>Repeat new password</span><input type="password" bind:value={next2} autocomplete="new-password" required /></label>
    {#if pwErr}<ErrorBox message={pwErr} />{/if}
    <div><button class="btn primary" type="submit">Change password</button></div>
  </form>

  <div class="card stack">
    <h2>Two-factor authentication {#if app.me.user.totp_enabled}<span class="badge ok">on</span>{/if}</h2>
    {#if app.me.user.totp_enabled}
      <p class="muted">Logins need a code from your authenticator app. Enter your password to turn it off.</p>
      <div class="row"><input type="password" bind:value={disablePw} placeholder="Password" style="max-width:240px" /><button class="btn danger" onclick={disable}>Turn off</button></div>
    {:else if enroll}
      <p>Scan this with an authenticator app (Aegis, 2FAS, Google Authenticator, 1Password…), then enter the 6-digit code.</p>
      <img src={enroll.qr} alt="QR code for your authenticator app" width="200" height="200" class="qr" />
      <p class="muted small">Can't scan? Enter this key manually: <code>{enroll.secret}</code></p>
      <div class="row"><input type="text" inputmode="numeric" bind:value={code} placeholder="123456" style="max-width:160px" /><button class="btn primary" onclick={enable}>Turn on</button></div>
    {:else}
      <p class="muted">Protect your account with a code from your phone in addition to your password.</p>
      <div><button class="btn" onclick={begin}>Set up two-factor</button></div>
    {/if}
    {#if tfErr}<ErrorBox message={tfErr} />{/if}
  </div>
</div>

<style>
  .qr { border-radius: 8px; background: #fff; padding: 6px; }
</style>
