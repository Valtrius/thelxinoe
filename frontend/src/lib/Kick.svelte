<script lang="ts">
  import { onMount } from 'svelte';
  import MediaGrid from './ui/MediaGrid.svelte';
  import LiveMediaCard from './ui/LiveMediaCard.svelte';
  import { api } from './api';
  import type { MediaChoice } from './playback';
  let { play } = $props<{ play: (choice: MediaChoice) => void }>();
  type Channel = {
    slug: string;
    title: string;
    category: string;
    live: boolean | null;
    viewers: number;
    thumbnail_url?: string;
    started_at?: string;
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

<details class="provider-account">
  <summary>Manage Kick channels</summary>
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
</details>
<section aria-label="Tracked Kick channels">
  <h2>Tracked channels</h2>
  <MediaGrid
    label="Kick channels"
    revision={feed?.items.map((s) => s.slug).join(',')}
  >
    {#each feed?.items ?? [] as item (item.slug)}<LiveMediaCard
        id={item.slug}
        platform="kick"
        name={item.slug}
        title={item.title}
        category={item.category}
        viewers={item.viewers}
        thumbnail={item.thumbnail_url}
        startedAt={item.started_at}
        live={item.live}
        error={item.error}
        play={() =>
          play({
            id: `kick:${item.slug}`,
            title: item.title || item.slug,
            kind: 'video',
          })}
        remove={() =>
          void act(
            `/online/kick/channels/${encodeURIComponent(item.slug)}`,
            'DELETE',
          )}
      />{:else}<p class="muted">Track a channel to see it here.</p>{/each}
  </MediaGrid>
</section>
