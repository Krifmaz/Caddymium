<script>
  import { onMount } from 'svelte'
  import {
    Globe, ShieldCheck, Braces, FileInput, Rocket, History as HistoryIcon, FileCode, Settings as SettingsIcon,
    Users as UsersIcon, Bell, ScrollText, Search, Menu, LogOut, Sun, Moon, Monitor, CircleCheck, CircleX, Info, ArrowRight, X,
    Puzzle, Signal, Shield,
  } from '@lucide/svelte'
  import { api, setCSRF, handleUnauthorized } from './lib/api.js'
  import { app, refreshStatus, can } from './lib/state.svelte.js'
  import { route, go } from './lib/router.svelte.js'
  import { theme, setTheme } from './lib/theme.svelte.js'
  import CommandPalette from './lib/CommandPalette.svelte'
  import Login from './pages/Login.svelte'
  import Welcome from './pages/Welcome.svelte'
  import Sites from './pages/Sites.svelte'
  import SiteEditor from './pages/SiteEditor.svelte'
  import Changes from './pages/Changes.svelte'
  import Caddyfile from './pages/Caddyfile.svelte'
  import History from './pages/History.svelte'
  import Domains from './pages/Domains.svelte'
  import Snippets from './pages/Snippets.svelte'
  import Import from './pages/Import.svelte'
  import Settings from './pages/Settings.svelte'
  import Users from './pages/Users.svelte'
  import Notifications from './pages/Notifications.svelte'
  import Audit from './pages/Audit.svelte'
  import Account from './pages/Account.svelte'
  import Plugins from './pages/Plugins.svelte'
  import Ddns from './pages/Ddns.svelte'
  import Security from './pages/Security.svelte'

  let loading = $state(true)
  let menuOpen = $state(false)
  let palette = $state()

  async function loadMe() {
    try {
      app.me = await api.get('/me')
      setCSRF(app.me.csrf)
    } catch (e) {
      app.me = { user: null, error: e.message }
    }
    loading = false
    if (app.me?.user && !app.me.pending_2fa) refreshStatus()
  }

  handleUnauthorized(() => { if (app.me?.user) loadMe() })

  onMount(() => {
    loadMe()
    const t = setInterval(() => { if (app.me?.user && !document.hidden) refreshStatus() }, 20000)
    return () => clearInterval(t)
  })

  async function logout() {
    await api.post('/logout').catch(() => {})
    app.me = { ...app.me, user: null, csrf: '' }
    setCSRF('')
    go('/')
  }

  const groups = [
    { label: 'Configure', items: [
      { path: '/sites', label: 'Sites', icon: Globe },
      { path: '/domains', label: 'Domains & DNS', icon: ShieldCheck, admin: true },
      { path: '/ddns', label: 'Dynamic DNS', icon: Signal, admin: true },
      { path: '/security', label: 'Security', icon: Shield, admin: true },
      { path: '/snippets', label: 'Snippets & raw', icon: Braces },
      { path: '/import', label: 'Import', icon: FileInput, admin: true },
    ] },
    { label: 'Deploy', items: [
      { path: '/changes', label: 'Review & apply', icon: Rocket, pending: true },
      { path: '/history', label: 'History', icon: HistoryIcon },
      { path: '/caddyfile', label: 'Caddyfile', icon: FileCode },
    ] },
    { label: 'Manage', items: [
      { path: '/settings', label: 'Settings', icon: SettingsIcon },
      { path: '/plugins', label: 'Plugins', icon: Puzzle, admin: true, update: true },
      { path: '/users', label: 'Users & API keys', icon: UsersIcon, admin: true },
      { path: '/notifications', label: 'Notifications', icon: Bell, admin: true },
      { path: '/audit', label: 'Audit log', icon: ScrollText, admin: true },
    ] },
  ]
  const visiblePages = $derived(groups.flatMap((g) => g.items).filter((n) => !n.admin || can.admin()))

  const page = $derived(route.parts[0] || 'sites')
  const showWelcome = $derived(app.me?.user && can.admin() && app.me.setup_complete === false && page !== 'import')
  const st = $derived(app.status)
  const isMac = navigator.platform.toLowerCase().includes('mac')
  const wdTime = (at) => {
    const d = new Date(at)
    const sameDay = d.toDateString() === new Date().toDateString()
    return sameDay ? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : d.toLocaleString([], { dateStyle: 'short', timeStyle: 'short' })
  }
  const toastIcon = { ok: CircleCheck, err: CircleX, info: Info }
</script>

{#if loading}
  <div class="boot"><img src="/favicon.svg" alt="" width="40" height="40" /></div>
{:else if !app.me?.user || app.me.pending_2fa}
  <Login onlogin={loadMe} />
{:else}
  <div class="shell">
    <aside class:open={menuOpen}>
      <div class="brand">
        <img src="/favicon.svg" alt="" width="28" height="28" />
        <span>Caddymium</span>
        <button class="btn ghost icon sm close-menu" aria-label="Close menu" onclick={() => (menuOpen = false)}><X size={18} /></button>
      </div>

      <button class="search" onclick={() => { menuOpen = false; palette.show() }}>
        <Search size={15} />
        <span class="grow">Search…</span>
        <span class="kbd">{isMac ? '⌘' : 'Ctrl'} K</span>
      </button>

      <nav>
        {#each groups as g}
          <div class="group">{g.label}</div>
          {#each g.items as n}
            {#if !n.admin || can.admin()}
              <a href={'#' + n.path} class:active={page === n.path.slice(1)} onclick={() => (menuOpen = false)}>
                <n.icon size={17} strokeWidth={1.9} />
                <span class="grow">{n.label}</span>
                {#if n.pending && st?.pending_changes}<span class="dot warn pulse" title="Unapplied changes"></span>{/if}
                {#if n.update && st?.update_available}<span class="dot warn" title="Caddy {st.latest_caddy} is available"></span>{/if}
              </a>
            {/if}
          {/each}
        {/each}
      </nav>

      <div class="foot">
        <div class="caddy">
          {#if st}
            {#if st.caddy_reachable}
              <span class="dot ok"></span><span class="grow">Caddy {st.caddy_version || ''}</span><span class="faint xs">connected</span>
            {:else}
              <span class="dot err"></span><span class="grow" title={st.caddy_error}>Caddy unreachable</span>
            {/if}
          {:else}
            <span class="dot"></span><span class="faint">Checking Caddy…</span>
          {/if}
        </div>
        {#if st?.watchdog && Date.now() - new Date(st.watchdog.at).getTime() < 86400000}
          <a class="watchdog xs" class:err={!st.watchdog.ok} href="#/plugins" title={`${st.watchdog.reason}\n${st.watchdog.outcome}`}>
            {st.watchdog.ok ? 'Caddy restarted by watchdog' : 'Watchdog could not restart Caddy'} at {wdTime(st.watchdog.at)}
          </a>
        {/if}
        <div class="me">
          <a href="#/account" class="me-link" class:active={page === 'account'} onclick={() => (menuOpen = false)}>
            <span class="avatar me-av">{app.me.user.username[0]?.toUpperCase()}</span>
            <span class="me-text">
              <span class="truncate">{app.me.user.username}</span>
              <span class="faint xs">{app.me.user.role}</span>
            </span>
          </a>
          <button class="btn ghost icon sm" title="Log out" aria-label="Log out" onclick={logout}><LogOut size={16} /></button>
        </div>
        <div class="theme" role="group" aria-label="Theme">
          {#each [['light', Sun], ['system', Monitor], ['dark', Moon]] as [t, Icon]}
            <button class:active={theme.choice === t} onclick={() => setTheme(t)} title={t} aria-label={t + ' theme'}><Icon size={14} /></button>
          {/each}
        </div>
      </div>
    </aside>
    {#if menuOpen}<div class="scrim" role="presentation" onclick={() => (menuOpen = false)}></div>{/if}

    <div class="main">
      <header class="mobile-top">
        <button class="btn ghost icon" aria-label="Menu" onclick={() => (menuOpen = true)}><Menu size={20} /></button>
        <img src="/favicon.svg" alt="" width="24" height="24" />
        <strong>Caddymium</strong>
        <button class="btn ghost icon right" aria-label="Search" onclick={() => palette.show()}><Search size={19} /></button>
      </header>

      <main>
        {#if showWelcome}
          <Welcome />
        {:else if page === 'sites' && route.parts[1]}
          {#key route.parts[1]}<SiteEditor id={route.parts[1]} />{/key}
        {:else if page === 'sites'}
          <Sites />
        {:else if page === 'changes'}
          <Changes />
        {:else if page === 'caddyfile'}
          <Caddyfile />
        {:else if page === 'history'}
          <History />
        {:else if page === 'domains'}
          <Domains />
        {:else if page === 'snippets'}
          <Snippets />
        {:else if page === 'import'}
          <Import />
        {:else if page === 'settings'}
          <Settings />
        {:else if page === 'users'}
          <Users />
        {:else if page === 'notifications'}
          <Notifications />
        {:else if page === 'audit'}
          <Audit />
        {:else if page === 'account'}
          <Account />
        {:else if page === 'plugins'}
          <Plugins />
        {:else if page === 'ddns'}
          <Ddns />
        {:else if page === 'security'}
          <Security />
        {:else}
          <div class="empty"><h2>Page not found</h2><a href="#/sites">Go to sites</a></div>
        {/if}
      </main>
    </div>
  </div>

  {#if st?.pending_changes && can.edit() && page !== 'changes' && !(page === 'sites' && route.parts[1]) && !showWelcome}
    <a class="pending" href="#/changes">
      <span class="dot warn pulse"></span>
      <span>You have unapplied changes</span>
      <span class="go">Review & apply <ArrowRight size={15} /></span>
    </a>
  {/if}

  <CommandPalette bind:this={palette} pages={visiblePages} />
{/if}

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    {@const Icon = toastIcon[t.kind] || Info}
    <div class="toast {t.kind}"><Icon size={18} /><span>{t.message}</span></div>
  {/each}
</div>

<style>
  .boot { display: grid; place-items: center; min-height: 100vh; min-height: 100dvh; animation: pulse 1.2s infinite; }
  .shell { display: flex; min-height: 100vh; min-height: 100dvh; }

  aside {
    width: 256px; flex: none; display: flex; flex-direction: column; gap: 4px;
    position: sticky; top: 0; height: 100vh; height: 100dvh; padding: 14px 12px max(14px, env(safe-area-inset-bottom));
    border-right: 1px solid var(--border); background: color-mix(in srgb, var(--surface) 70%, transparent);
    backdrop-filter: blur(8px);
  }
  .brand { display: flex; align-items: center; gap: 10px; padding: 4px 8px 12px; font-weight: 700; font-size: 16px; letter-spacing: -0.02em; }
  .close-menu { display: none; margin-left: auto; }
  .search {
    display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 8px 0 10px; margin: 0 2px 8px;
    border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--surface); color: var(--muted);
    font: inherit; font-size: 13px; cursor: pointer; box-shadow: var(--shadow-xs); text-align: left;
  }
  .search:hover { border-color: var(--border-strong); color: var(--text-2); }

  .watchdog { display: block; padding: 2px 10px 6px; color: var(--warn-soft-text, var(--muted)); text-decoration: none; }
  .watchdog.err { color: var(--err, #ef4444); }
  .watchdog:hover { text-decoration: underline; }
  nav { display: flex; flex-direction: column; gap: 1px; overflow-y: auto; flex: 1; padding: 0 2px; }
  .group { font-size: 11px; font-weight: 600; color: var(--faint); text-transform: uppercase; letter-spacing: 0.06em; padding: 14px 10px 6px; }
  .group:first-child { padding-top: 4px; }
  nav a {
    display: flex; align-items: center; gap: 10px; height: 34px; padding: 0 10px; border-radius: var(--radius-sm);
    color: var(--text-2); font-weight: 500; font-size: 13.5px; transition: background 0.12s, color 0.12s;
  }
  nav a :global(svg) { color: var(--muted); flex: none; }
  nav a:hover { background: var(--surface-2); color: var(--text); }
  nav a.active { background: var(--surface); color: var(--text); font-weight: 600; box-shadow: var(--shadow-sm); border: 1px solid var(--border); }
  nav a.active :global(svg) { color: var(--accent); }

  .foot { display: flex; flex-direction: column; gap: 8px; padding: 10px 2px 0; border-top: 1px solid var(--border); }
  .caddy { display: flex; align-items: center; gap: 9px; font-size: 12.5px; padding: 4px 10px; color: var(--text-2); }
  .me { display: flex; align-items: center; gap: 4px; }
  .me-link { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; padding: 6px 8px; border-radius: var(--radius-sm); color: var(--text); }
  .me-link:hover, .me-link.active { background: var(--surface-2); color: var(--text); }
  .me-av { width: 30px; height: 30px; border-radius: 8px; font-size: 13px; background: linear-gradient(135deg, var(--accent), #0e7490); }
  .me-text { display: flex; flex-direction: column; min-width: 0; line-height: 1.3; font-weight: 550; font-size: 13px; }
  .theme { display: flex; gap: 2px; padding: 3px; background: var(--surface-2); border: 1px solid var(--border); border-radius: 9px; }
  .theme button {
    flex: 1; height: 26px; display: grid; place-items: center; border: 0; border-radius: 6px; background: none;
    color: var(--muted); cursor: pointer;
  }
  .theme button:hover { color: var(--text); }
  .theme button.active { background: var(--surface); color: var(--text); box-shadow: var(--shadow-sm); }

  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .mobile-top { display: none; }
  main { padding: 36px 40px 120px; max-width: 1200px; width: 100%; margin: 0 auto; animation: enter 0.2s ease-out; }
  .scrim { display: none; }

  .pending {
    position: fixed; bottom: 22px; left: calc(50% + 128px); transform: translateX(-50%); z-index: 20;
    display: flex; align-items: center; gap: 12px; padding: 7px 7px 7px 16px; border-radius: 999px;
    background: var(--text); color: var(--bg); box-shadow: var(--shadow-lg); font-size: 13.5px; font-weight: 500;
    white-space: nowrap; animation: rise 0.25s ease-out;
  }
  .pending:hover { color: var(--bg); }
  .pending .go {
    display: inline-flex; align-items: center; gap: 6px; padding: 6px 12px; border-radius: 999px;
    background: var(--accent); color: var(--accent-text); font-weight: 600;
  }

  @media (max-width: 900px) {
    aside { position: fixed; left: 0; top: 0; z-index: 40; transform: translateX(-100%); transition: transform 0.2s ease; background: var(--surface); }
    aside.open { transform: none; box-shadow: var(--shadow-lg); }
    .close-menu { display: inline-flex; }
    .scrim { display: block; position: fixed; inset: 0; z-index: 30; background: rgba(9, 9, 11, 0.4); backdrop-filter: blur(2px); }
    .mobile-top {
      display: flex; align-items: center; gap: 8px; height: 56px; padding: 0 8px; position: sticky; top: 0; z-index: 10;
      background: color-mix(in srgb, var(--surface) 85%, transparent); backdrop-filter: blur(10px); border-bottom: 1px solid var(--border);
    }
    main { padding: 20px 16px 110px; }
    .pending { left: 50%; bottom: 16px; }
  }
  @media (max-width: 420px) {
    .pending > span:nth-child(2) { display: none; }
  }

  .toasts { position: fixed; top: 16px; right: 16px; left: 16px; display: flex; flex-direction: column; align-items: flex-end; gap: 8px; z-index: 100; pointer-events: none; }
  .toast {
    pointer-events: auto; max-width: 420px; display: flex; gap: 10px; align-items: flex-start;
    padding: 12px 14px; border-radius: var(--radius); background: var(--surface); color: var(--text);
    border: 1px solid var(--border); box-shadow: var(--shadow-lg); white-space: pre-wrap; word-break: break-word;
    font-size: 13.5px; animation: slide 0.22s ease-out;
  }
  .toast :global(svg) { flex: none; margin-top: 1px; }
  .toast.ok :global(svg) { color: var(--ok); }
  .toast.err :global(svg) { color: var(--danger); }
  @keyframes slide { from { opacity: 0; transform: translateY(-8px) scale(0.98); } }
  @keyframes rise { from { opacity: 0; transform: translate(-50%, 12px); } }
  @keyframes enter { from { opacity: 0; transform: translateY(4px); } }
  @keyframes pulse { 50% { opacity: 0.5; } }
</style>
