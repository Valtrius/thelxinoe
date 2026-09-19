<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import type { MediaChoice } from './playback';
  let { play } = $props<{ play: (choice: MediaChoice) => void }>();
  type Channel = {
    slug: string;
    title: string;
    category: string;
    live: boolean | null;
    viewers: number;
    error: string | null;
  };
  type Feed = { connected: boolean; configured: boolean; items: Channel[] };
  let feed = $state<Feed | null>(null),
    channel = $state(''),
    error = $state(''),
    busy = $state(false),
    deleting = $state(false);
  let disposed = false,
    loading = false;
  async function load() {
    if (loading) return;
    loading = true;
    try {
      const value = await api<Feed>('/online/kick');
      if (!disposed) feed = value;
    } finally {
      loading = false;
    }
  }
  async function act(path: string, method: string, body?: unknown) {
    busy = true;
    error = '';
    try {
      await api(path, method, body);
      await load();
      deleting = false;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function add() {
    const value = channel.trim();
    if (!value) return;
    channel = '';
    await act('/online/kick/channels', 'POST', { channel: value });
  }
  onMount(() => {
    disposed = false;
    void load().catch((e) => (error = String(e)));
    const timer = setInterval(() => {
      void load().catch((e) => (error = String(e)));
    }, 5000);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  });
</script>

<section class="panel">
  <h2>Kick</h2>
  <p>
    Track channels on this server. A Kick account is not needed for public
    playback.
  </p>
  {#if error}<p role="alert">{error}</p>{/if}
  {#if feed && !feed.configured}<p>
      Live status and titles need the administrator's Kick application
      credentials. You can still track channels and try public playback.
    </p>{/if}
  <form
    class="inline-form"
    onsubmit={(e) => {
      e.preventDefault();
      void add();
    }}
  >
    <label
      >Kick channel<input
        bind:value={channel}
        placeholder="Channel name or https://kick.com/channel"
        required
      /></label
    ><button class="primary" disabled={busy}>Track channel</button>
  </form>
  {#if feed?.connected}<button
      disabled={busy}
      onclick={() => void act('/online/kick', 'DELETE')}
      >Disconnect Kick synchronization</button
    >{:else}<button
      disabled={busy}
      onclick={() => void act('/online/kick/connect', 'POST')}
      >Resume Kick synchronization</button
    >{/if}
  <button disabled={busy} onclick={() => (deleting = !deleting)}
    >Delete Kick data</button
  >
  {#if deleting}<p>
      Remove your tracked channels and Kick playback history from this server?
    </p>
    <button
      disabled={busy}
      onclick={() => void act('/online/kick/data', 'DELETE')}
      >Confirm delete Kick data</button
    >{/if}
</section>
<section aria-label="Tracked Kick channels">
  {#each feed?.items ?? [] as item (item.slug)}
    <article class="panel">
      <h3>{item.slug}</h3>
      <p>{item.title}</p>
      <p class="muted">
        {item.live === null ? 'Status unknown' : item.live ? 'Live' : 'Offline'} ·
        {item.category}{item.live
          ? ` · ${item.viewers.toLocaleString()} viewers`
          : ''}
      </p>
      <button
        class="primary"
        aria-label={`Play ${item.slug}`}
        onclick={() =>
          play({
            id: `kick:${item.slug}`,
            title: item.title || item.slug,
            kind: 'video',
          })}>Play live</button
      >
      <button
        disabled={busy}
        aria-label={`Stop tracking ${item.slug}`}
        onclick={() =>
          void act(
            `/online/kick/channels/${encodeURIComponent(item.slug)}`,
            'DELETE',
          )}>Stop tracking</button
      >
      {#if item.error}<p role="status">{item.error}</p>{/if}
    </article>
  {/each}
</section>
