<script lang="ts">
  import { onMount } from 'svelte';
  import MediaGrid from './ui/MediaGrid.svelte';
  import LiveMediaCard from './ui/LiveMediaCard.svelte';
  import { api, desktop } from './api';
  import { invoke } from '@tauri-apps/api/core';
  import type { MediaChoice } from './playback';
  let { play } = $props<{ play: (choice: MediaChoice) => void }>();
  type Account = {
    configured: boolean;
    account: { status: string; display_name: string };
    pending: null | {
      user_code: string;
      verification_uri: string;
      expires_at: number;
      error: string | null;
    };
    sync: null | {
      last_complete: number | null;
      next_run: number;
      error: string | null;
    };
  };
  type Stream = {
    id: string;
    login: string;
    display_name: string;
    title: string;
    category: string;
    viewers: number;
    thumbnail_url?: string;
    started_at?: string;
  };
  let account = $state<Account | null>(null),
    streams = $state<Stream[]>([]),
    busy = $state(false),
    error = $state(''),
    deleting = $state(false);
  let disposed = false,
    loading = false;
  async function load() {
    if (loading) return;
    loading = true;
    try {
      const [a, f] = await Promise.all([
        api<Account>('/online/twitch'),
        api<{ items: Stream[] }>('/online/twitch/feed'),
      ]);
      if (!disposed) {
        account = a;
        streams = f.items;
      }
    } finally {
      loading = false;
    }
  }
  async function act(path: string, method: string) {
    busy = true;
    error = '';
    try {
      await api(path, method);
      await load();
      deleting = false;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function authorize() {
    if (!account?.pending) return;
    const url = account.pending.verification_uri;
    if (desktop) {
      try {
        await invoke('open_twitch_activation');
      } catch (e) {
        error = String(e);
      }
    } else window.open(url, '_blank', 'noopener,noreferrer');
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

<details
  class="provider-account"
  open={account?.account.status !== 'connected'}
>
  <summary
    >{account?.account.status === 'connected'
      ? `Connected as ${account.account.display_name}`
      : 'Your Twitch account'}</summary
  >
  {#if error}<p role="alert">{error}</p>{/if}
  {#if account}
    <p>
      {account.account.status === 'connected'
        ? `Connected as ${account.account.display_name}`
        : account.account.status === 'reconnect_required'
          ? 'Reconnect Twitch to resume synchronization.'
          : 'Connect Twitch to see your followed live channels.'}
    </p>
    {#if !account.configured}<p>
        An administrator must configure the Twitch application in Settings.
      </p>{/if}
    {#if account.pending}
      <div class="panel" aria-label="Twitch authorization">
        <p>
          Enter this code on Twitch, then approve access to your followed
          channels:
        </p>
        <strong class="device-code">{account.pending.user_code}</strong>
        <p>
          <button class="primary" onclick={() => void authorize()}
            >Open Twitch activation</button
          >
        </p>
        <p class="muted">
          Waiting for your approval. This code expires at {new Date(
            account.pending.expires_at * 1000,
          ).toLocaleTimeString()}.
        </p>
        {#if account.pending.error}<p role="status">
            {account.pending.error}
          </p>{/if}
      </div>
    {:else}
      <button
        class="primary"
        disabled={busy || !account.configured}
        onclick={() => void act('/online/twitch/connect', 'POST')}
        >{account.account.status === 'connected'
          ? 'Reconnect Twitch'
          : 'Connect Twitch'}</button
      >
    {/if}
    {#if account.account.status !== 'disconnected' || account.pending}<button
        disabled={busy}
        onclick={() => void act('/online/twitch', 'DELETE')}
        >Disconnect Twitch</button
      >{/if}
    <button disabled={busy} onclick={() => (deleting = !deleting)}
      >Delete Twitch data</button
    >
    {#if deleting}<p>
        Disconnect Twitch and remove your cached channels from this server?
      </p>
      <button
        disabled={busy}
        onclick={() => void act('/online/twitch/data', 'DELETE')}
        >Confirm delete Twitch data</button
      >{/if}
    {#if account.sync?.error}<p role="status">{account.sync.error}</p>{/if}
    {#if account.sync?.last_complete}<p class="muted">
        Updated {new Date(
          account.sync.last_complete * 1000,
        ).toLocaleTimeString()}.
      </p>{/if}
  {/if}
</details>
<section aria-label="Followed live Twitch channels">
  <h2>Followed live channels</h2>
  {#if streams.length === 0}<p>
      No live channels in the latest completed synchronization.
    </p>{/if}
  <MediaGrid
    label="Live Twitch channels"
    revision={streams.map((s) => s.id).join(',')}
  >
    {#each streams as stream (stream.id)}<LiveMediaCard
        id={stream.id}
        platform="twitch"
        name={stream.display_name}
        title={stream.title}
        category={stream.category}
        viewers={stream.viewers}
        thumbnail={stream.thumbnail_url}
        startedAt={stream.started_at}
        play={() =>
          play({
            id: `twitch:${stream.id}`,
            title: stream.title,
            kind: 'video',
          })}
      />{/each}
  </MediaGrid>
</section>

<style>
  .device-code {
    font-size: 1.8rem;
    letter-spacing: 0.15em;
  }
</style>
