<script lang="ts">
  import { Bell, Check, Clock } from '@lucide/svelte';
  import { api, type User } from '../api';
  import { showToast } from '../providers/toasts';
  import Button from '../ui/Button.svelte';
  import {
    monitorLabel,
    requestMusic,
    scoped,
    type Artist,
    type MusicService,
  } from './types';
  let {
    artist,
    service,
    user,
  }: {
    /** Updated in place as monitoring changes. */
    artist: Artist;
    service: MusicService;
    user: User;
  } = $props();
  let busy = $state(false);
  const current = $derived(artist.request?.state);
  // Lidarr decides once a request has reached it; one on its way counts already.
  const monitoring = $derived(
    artist.monitored ||
      (current && ['approved', 'adding', 'searching'].includes(current)),
  );

  async function run(action: () => Promise<void>) {
    busy = true;
    try {
      await action();
    } catch (caught) {
      showToast({
        key: `monitor:${artist.id}`,
        tone: 'error',
        title: artist.name,
        message: String(caught),
      });
    } finally {
      busy = false;
    }
  }
  const monitor = () =>
    run(async () => {
      artist.request = await requestMusic(service, artist.id, 'artist');
    });
  const stop = () =>
    run(async () => {
      await api(
        scoped(
          `/acquisition/music/artists/${encodeURIComponent(artist.id)}/monitor`,
          service,
        ),
        'DELETE',
      );
      artist.monitored = false;
      artist.request = null;
    });
  const status =
    'inline-flex min-h-8.5 items-center gap-2 border border-line-strong bg-accent-soft px-3.25 py-2 text-xs font-semibold text-accent';
</script>

{#if monitoring && artist.monitored && user.role === 'admin'}<Button
    size="form"
    aria-pressed="true"
    loading={busy}
    title="Stop monitoring new releases"
    onclick={() => void stop()}
    ><Check size={15} strokeWidth={2.4} />Monitoring new releases</Button
  >{:else if monitoring}<span class={status}
    ><Check size={15} strokeWidth={2.4} />Monitoring new releases</span
  >{:else if current === 'pending'}<span class={status}
    ><Clock size={15} />{monitorLabel(current)}</span
  >{:else}<Button
    variant="secondary"
    size="form"
    aria-pressed="false"
    loading={busy}
    onclick={() => void monitor()}
    ><Bell size={15} />Monitor new releases</Button
  >{/if}
