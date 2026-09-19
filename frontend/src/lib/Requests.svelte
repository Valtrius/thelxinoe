<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type User } from './api';
  import RequestStatus from './RequestStatus.svelte';
  let { user, domain } = $props<{
    user: Pick<User, 'id' | 'role'>;
    domain?: string;
  }>();
  const managerKind = $derived(
    domain === 'Movies'
      ? 'radarr'
      : domain === 'Shows'
        ? 'sonarr'
        : domain === 'Music'
          ? 'lidarr'
          : null,
  );
  type Service = { id: string; name: string; kind: string; ready: boolean };
  type Item = {
    external_id: string;
    title: string;
    year: number | null;
    overview: string;
    artist: string | null;
  };
  type Request = {
    id: string;
    title: string;
    state: string;
    username: string;
    user_id: string;
    service: string;
    error: string | null;
    kind: string;
  };
  let services = $state<Service[]>([]),
    selected = $state(''),
    term = $state(''),
    items = $state<Item[]>([]),
    local = $state<{ id: string; title: string; year: number | null }[]>([]),
    requests = $state<Request[]>([]),
    error = $state(''),
    notice = $state(''),
    busy = $state(false);
  let disposed = false;
  async function refresh() {
    const result = await api<{ items: Request[] }>('/acquisition/requests');
    if (!disposed)
      requests = result.items.filter(
        (r) => !managerKind || r.kind === managerKind,
      );
  }
  async function act(fn: () => Promise<void>) {
    busy = true;
    error = '';
    notice = '';
    try {
      await fn();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function search() {
    const result = await api<{ items: Item[]; local: typeof local }>(
      `/acquisition/search?service_id=${encodeURIComponent(selected)}&term=${encodeURIComponent(term.trim())}`,
    );
    items = result.items;
    local = result.local;
  }
  async function request(item: Item) {
    const result = await api<{ state: string }>(
      '/acquisition/requests',
      'POST',
      { service_id: selected, external_id: item.external_id },
    );
    notice =
      result.state === 'pending'
        ? 'Request sent for administrator approval.'
        : 'Request queued with the acquisition manager.';
    await refresh();
  }
  async function decide(id: string, action: string) {
    await api(`/acquisition/requests/${id}`, 'POST', { action });
    await refresh();
  }
  onMount(() => {
    disposed = false;
    void act(async () => {
      services = (
        await api<{ items: Service[] }>('/acquisition/services')
      ).items.filter((s) => !managerKind || s.kind === managerKind);
      selected = services[0]?.id ?? '';
      await refresh();
    });
    const timer = setInterval(() => {
      void refresh().catch((e) => (error = String(e)));
    }, 5000);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  });
</script>

<section class="panel">
  <h2>Requests</h2>
  <p>
    Search your library and the connected acquisition services. Requests need
    administrator approval unless your account has auto-approval.
  </p>
  {#if error}<p role="alert">{error}</p>{/if}{#if notice}<p role="status">
      {notice}
    </p>{/if}
  {#if !services.length}<p>
      An administrator must connect an acquisition manager in Settings.
    </p>{:else}
    <form
      class="inline-form"
      onsubmit={(e) => {
        e.preventDefault();
        void act(search);
      }}
    >
      <label
        >Acquisition service<select
          bind:value={selected}
          onchange={() => {
            items = [];
            local = [];
          }}
          >{#each services as s (s.id)}<option value={s.id}
              >{s.name} ({s.kind})</option
            >{/each}</select
        ></label
      ><label
        >Search media<input
          bind:value={term}
          required
          minlength="2"
          maxlength="200"
        /></label
      ><button class="primary" disabled={busy}>Search media</button>
    </form>
  {/if}
</section>
{#if local.length}<section class="panel">
    <h3>In your library</h3>
    {#each local as item (item.id)}<p>{item.title} {item.year ?? ''}</p>{/each}
  </section>{/if}
<section aria-label="Acquisition search results">
  {#each items as item (item.external_id)}<article class="panel">
      <h3>{item.title} {item.year ?? ''}</h3>
      {#if item.artist}<p>{item.artist}</p>{/if}
      <p class="muted">{item.overview}</p>
      <button
        disabled={busy || !services.find((s) => s.id === selected)?.ready}
        aria-label={`Request ${item.title}`}
        onclick={() => void act(() => request(item))}>Request</button
      >
    </article>{/each}
</section>
<section aria-label="Media requests">
  <h2>{user.role === 'admin' ? 'All requests' : 'Your requests'}</h2>
  {#each requests as request (request.id)}<article class="panel">
      <h3>{request.title}</h3>
      <p>
        {request.state} · {request.service}{user.role === 'admin'
          ? ` · ${request.username}`
          : ''}
      </p>
      {#if request.error}<p role="status">{request.error}</p>{/if}
      {#if ['requested', 'available'].includes(request.state)}<RequestStatus
          id={request.id}
          admin={user.role === 'admin'}
        /><button
          disabled={busy}
          onclick={() => void act(() => decide(request.id, 'reacquire'))}
          >Request again</button
        >{/if}
      {#if ['pending', 'failed', 'uncertain'].includes(request.state)}
        {#if user.role === 'admin'}<button
            disabled={busy}
            onclick={() => void act(() => decide(request.id, 'approve'))}
            >{request.state === 'pending'
              ? 'Approve'
              : 'Retry after review'}</button
          ><button
            disabled={busy}
            onclick={() => void act(() => decide(request.id, 'deny'))}
            >Deny</button
          >{/if}
        {#if request.user_id === user.id}<button
            disabled={busy}
            onclick={() => void act(() => decide(request.id, 'cancel'))}
            >Cancel request</button
          >{/if}
      {/if}
    </article>{/each}
</section>
