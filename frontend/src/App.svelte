<script lang="ts">
  import { onMount } from 'svelte';
  import LibraryView from './lib/LibraryView.svelte';
  import MetadataSettings from './lib/MetadataSettings.svelte';
  import {
    House,
    Film,
    Tv,
    Music,
    Play,
    Radio,
    Settings,
    LogOut,
    Library,
    ShieldCheck,
    RefreshCw,
  } from '@lucide/svelte';
  import {
    api,
    Events,
    type User,
    desktop,
    initializeTransport,
    changeServer,
    serverUrl,
  } from './lib/api';
  type Session = {
    id: string;
    name: string;
    transport: string;
    last_seen: number;
  };
  type Job = { id: string; kind: string; state: string; error: string | null };
  let loading = $state(true),
    setup = $state(false),
    user = $state<User | null>(null),
    error = $state(''),
    busy = $state(false),
    connected = $state(false);
  let username = $state(''),
    password = $state(''),
    setupToken = $state(''),
    section = $state('Home');
  let sessions = $state<Session[]>([]),
    users = $state<User[]>([]),
    jobs = $state<Job[]>([]),
    currentSession = $state('');
  let newUsername = $state(''),
    newPassword = $state(''),
    newRole = $state<'admin' | 'user'>('user');
  let timezone = $state('UTC'),
    health = $state<{
      version: string;
      controller: boolean;
      cache_free_bytes: number;
    } | null>(null);
  const sections = [
    { name: 'Home', icon: House },
    { name: 'Movies', icon: Film },
    { name: 'Shows', icon: Tv },
    { name: 'Music', icon: Music },
    { name: 'YouTube', icon: Play },
    { name: 'Twitch', icon: Radio },
    { name: 'Kick', icon: Radio },
  ];
  let events: Events | undefined;
  let catalogRevision = $state(0);
  let scans = $state<Record<string, { completed: number; total: number }>>({});
  let serverAddress = $state('');
  async function act(fn: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await fn();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  async function boot() {
    try {
      await initializeTransport();
      serverAddress = serverUrl();
      setup = (await api<{ setup_required: boolean }>('/setup')).setup_required;
      if (!setup) {
        try {
          user = (await api<{ user: User }>('/auth/me')).user;
        } catch {
          user = null;
        }
      }
      if (user) startEvents();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }
  function startEvents() {
    events?.close();
    events = new Events(
      (event) => {
        if (event.kind === 'catalog.changed') {
          catalogRevision++;
          const root = (event.payload as { root_id?: string }).root_id;
          if (root) delete scans[root];
        }
        if (event.kind === 'catalog.scan.progress') {
          const scan = event.payload as {
            root_id: string;
            completed: number;
            total: number;
          };
          scans[scan.root_id] = scan;
        }
        if (section === 'Settings') void loadSettings();
      },
      (value) => (connected = value),
    );
    events.connect();
  }
  async function authenticate() {
    await act(async () => {
      user = (
        await api<{ user: User }>(setup ? '/setup' : '/auth/login', 'POST', {
          username,
          password,
          setup_token: setupToken,
        })
      ).user;
      password = '';
      setupToken = '';
      setup = false;
      startEvents();
    });
  }
  async function logout() {
    await act(async () => {
      await api('/auth/logout', 'POST');
      user = null;
      events?.close();
    });
  }
  async function loadSettings() {
    await act(async () => {
      const result = await api<{ items: Session[]; current: string }>(
        '/auth/sessions',
      );
      sessions = result.items;
      currentSession = result.current;
      if (user?.role === 'admin') {
        users = (await api<{ items: User[] }>('/users')).items;
        jobs = (await api<{ items: Job[] }>('/admin/jobs')).items;
        health = await api('/admin/health');
        timezone = (await api<{ timezone: string }>('/admin/settings'))
          .timezone;
      }
    });
  }
  async function navigate(name: string) {
    section = name;
    error = '';
    if (name === 'Settings') await loadSettings();
  }
  async function createUser() {
    await act(async () => {
      await api('/users', 'POST', {
        username: newUsername,
        password: newPassword,
        role: newRole,
      });
      newUsername = '';
      newPassword = '';
      users = (await api<{ items: User[] }>('/users')).items;
    });
  }
  onMount(() => {
    void boot();
    return () => events?.close();
  });
</script>

{#if loading}
  <main class="auth-page">
    <div class="brand-mark">T</div>
    <p>Connecting to your library…</p>
  </main>
{:else if !user}
  <main class="auth-page">
    <div class="auth-card">
      <div class="brand-mark">T</div>
      <p class="eyebrow">YOUR MEDIA. YOUR PLACE.</p>
      <h1>{setup ? 'Welcome to Thelxinoe' : 'Welcome back'}</h1>
      <p class="muted">
        {setup
          ? 'Create the administrator account for your media server.'
          : 'Sign in to pick up where you left off.'}
      </p>
      {#if desktop}<label
          >Server address<input
            bind:value={serverAddress}
            placeholder="https://media.example.com"
          /></label
        ><button
          type="button"
          class="secondary"
          onclick={() =>
            act(async () => {
              await changeServer(serverAddress);
              await boot();
            })}>Connect to server</button
        >{/if}
      <form
        onsubmit={(event) => {
          event.preventDefault();
          void authenticate();
        }}
      >
        {#if setup}<label
            >Setup code<input
              bind:value={setupToken}
              required
              autocomplete="off"
              placeholder="From your server’s secrets/setup-token file"
            /></label
          >{/if}
        <label
          >Username<input
            bind:value={username}
            required
            autocomplete="username"
          /></label
        >
        <label
          >Password<input
            bind:value={password}
            type="password"
            required
            minlength={setup ? 12 : 1}
            autocomplete={setup ? 'new-password' : 'current-password'}
          /></label
        >
        {#if setup}<p class="hint">
            Use at least 12 characters. You can create other users after setup.
          </p>{/if}
        {#if error}<p role="alert" class="error">{error}</p>{/if}
        <button class="primary" disabled={busy}
          >{busy
            ? 'Connecting…'
            : setup
              ? 'Create your server'
              : 'Sign in'}</button
        >
      </form>
      <p class="footnote">A shared library, a space of your own.</p>
    </div>
  </main>
{:else}
  <div class="app-shell">
    <aside>
      <a class="brand" href="#home" onclick={() => navigate('Home')}
        ><span class="brand-mark small">T</span>Thelxinoe</a
      >
      <p class="nav-label">YOUR LIBRARY</p>
      <nav aria-label="Main navigation">
        {#each sections as item (item.name)}<button
            class:active={section === item.name}
            onclick={() => navigate(item.name)}
            ><item.icon size={19} /><span>{item.name}</span></button
          >{/each}
      </nav>
      <div class="sidebar-bottom">
        <button
          class:active={section === 'Settings'}
          onclick={() => navigate('Settings')}
          ><Settings size={19} />Settings</button
        >
        <div class="profile">
          <span class="avatar">{user.username.slice(0, 1).toUpperCase()}</span>
          <div>
            <strong>{user.username}</strong><small
              >{user.role === 'admin' ? 'Administrator' : 'Member'}</small
            >
          </div>
          <button
            class="icon-button"
            title="Sign out"
            aria-label="Sign out"
            onclick={logout}><LogOut size={17} /></button
          >
        </div>
      </div>
    </aside>
    <main class="content">
      <header>
        <div>
          <p class="eyebrow">YOUR PERSONAL MEDIA SERVER</p>
          <h1>
            {section === 'Home' ? `Good to see you, ${user.username}` : section}
          </h1>
        </div>
        <span class="connection"
          ><i class:online={connected}></i>{connected
            ? 'Connected'
            : 'Reconnecting'}</span
        >
      </header>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      {#if section === 'Settings'}
        <section class="panel">
          <h2>Your devices</h2>
          <p class="muted">
            Revoke access to a browser or desktop at any time.
          </p>
          {#each sessions as session (session.id)}<div class="row">
              <div>
                <strong>{session.name}</strong><small
                  >{session.transport} · {new Date(
                    session.last_seen * 1000,
                  ).toLocaleString()}{session.id === currentSession
                    ? ' · This device'
                    : ''}</small
                >
              </div>
              <button
                class="secondary"
                disabled={busy}
                onclick={() =>
                  act(async () => {
                    await api(`/auth/sessions/${session.id}`, 'DELETE');
                    if (session.id === currentSession) {
                      user = null;
                      events?.close();
                    } else await loadSettings();
                  })}>Revoke</button
              >
            </div>{/each}
        </section>
        {#if user.role === 'admin'}
          <MetadataSettings />
          <section class="panel">
            <h2><ShieldCheck size={20} /> Server</h2>
            <div class="stats">
              <div>
                <strong>{health?.version ?? '—'}</strong><small
                  >Product version</small
                >
              </div>
              <div>
                <strong
                  >{health?.controller ? 'Connected' : 'Unavailable'}</strong
                ><small>Docker controller</small>
              </div>
              <div>
                <strong
                  >{health
                    ? `${(health.cache_free_bytes / 1024 ** 3).toFixed(1)} GB`
                    : '—'}</strong
                ><small>Cache space available</small>
              </div>
            </div>
            <form
              class="inline-form"
              onsubmit={(e) => {
                e.preventDefault();
                void act(async () => {
                  await api('/admin/settings', 'PUT', { timezone });
                });
              }}
            >
              <label
                >Server timezone<input
                  bind:value={timezone}
                  placeholder="Europe/Paris"
                /></label
              ><button class="secondary" disabled={busy}>Save</button>
            </form>
          </section>
          <section class="panel">
            <h2>People</h2>
            {#each users as person (person.id)}<div class="row">
                <strong>{person.username}</strong><span class="badge"
                  >{person.role}</span
                >
              </div>{/each}
            <form
              class="inline-form"
              onsubmit={(e) => {
                e.preventDefault();
                void createUser();
              }}
            >
              <label
                >Username<input
                  bind:value={newUsername}
                  required
                  autocomplete="off"
                /></label
              ><label
                >Password<input
                  bind:value={newPassword}
                  type="password"
                  required
                  minlength="12"
                  autocomplete="new-password"
                /></label
              ><label
                >Role<select bind:value={newRole}
                  ><option value="user">User</option><option value="admin"
                    >Administrator</option
                  ></select
                ></label
              ><button class="primary" disabled={busy}>Add user</button>
            </form>
          </section>
          <section class="panel">
            <div class="section-heading">
              <h2>Background jobs</h2>
              <button
                class="secondary"
                onclick={() =>
                  act(async () => {
                    await api('/admin/jobs', 'POST', {
                      key: crypto.randomUUID(),
                    });
                    await loadSettings();
                  })}><RefreshCw size={15} /> Run checkpoint</button
              >
            </div>
            {#each jobs as job (job.id)}<div class="row">
                <span>{job.kind}</span><span class="badge">{job.state}</span>
              </div>{:else}<p class="muted">No background jobs yet.</p>{/each}
          </section>
        {/if}
      {:else if section === 'Home'}
        <section class="welcome">
          <div>
            <p class="eyebrow">MAKE YOURSELF AT HOME</p>
            <h2>One home for everything<br />you love to watch and hear.</h2>
            <p>
              Movies, series, music, and your favorite creators.<br />All
              together, with your place always saved.
            </p>
            <button class="primary" onclick={() => navigate('Movies')}
              >Explore your library</button
            >
          </div>
          <div class="welcome-art" aria-hidden="true">
            <Film size={64} /><Music size={50} /><Tv size={60} />
          </div>
        </section>
        <div class="section-heading">
          <h2>Your collections</h2>
          <span class="muted">Built around you</span>
        </div>
        <div class="domain-grid">
          {#each sections.slice(1, 4) as item (item.name)}<button
              class="domain-card"
              onclick={() => navigate(item.name)}
              ><item.icon size={30} /><strong>{item.name}</strong><span
                >Browse your collection →</span
              ></button
            >{/each}
        </div>
      {:else if ['Movies', 'Shows', 'Music'].includes(section)}
        <LibraryView
          domain={section}
          admin={user.role === 'admin'}
          revision={catalogRevision}
          {scans}
        />
      {:else}
        <section class="empty">
          <Library size={42} />
          <h2>
            {['Movies', 'Shows', 'Music'].includes(section)
              ? 'Your library starts here'
              : 'Your creators, together'}
          </h2>
          <p>
            {['Movies', 'Shows', 'Music'].includes(section)
              ? 'Add a library folder to discover your media.'
              : 'Connect your account to bring your channels and watchlists to Thelxinoe.'}
          </p>
          <span class="badge">This section is being implemented</span>
        </section>
      {/if}
    </main>
  </div>
{/if}
