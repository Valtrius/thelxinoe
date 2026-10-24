<script lang="ts">
  import { onMount } from 'svelte';
  import { Check } from '@lucide/svelte';
  import Modal from '../../../ui/Modal.svelte';
  import { api, normalizeError } from '../../api';
  import { showToast } from '../../toasts';
  import { takeSharedYoutubeVideo } from '../../youtube-share';
  import type { YoutubeWatchlist } from '../../types';

  let {
    available,
    openWatchlist,
  }: { available: boolean; openWatchlist: (id: number) => void } = $props();
  let videoId = '';
  let choices = $state<YoutubeWatchlist[]>([]);
  let busy = $state(false);

  function failed(caught: unknown) {
    showToast({
      key: 'youtube-share',
      tone: 'error',
      title: 'Could not save the shared video',
      message: normalizeError(caught).message,
      actionLabel: 'Retry',
      onAction: () => void offer(),
      durationMs: null,
    });
  }

  async function save(watchlist: YoutubeWatchlist) {
    busy = true;
    try {
      const result = await api.addYoutubeVideoToWatchlist(
        watchlist.id,
        videoId,
        Math.max(-1, ...watchlist.items.map((item) => item.manualPosition)) + 1,
      );
      showToast({
        key: 'youtube-share',
        tone: result.autoDownloadError ? 'warning' : 'success',
        title: result.autoDownloadError
          ? 'Saved, download not started'
          : result.added
            ? 'Video saved'
            : 'Already saved',
        message:
          result.autoDownloadError?.message ??
          `${result.added ? 'Added to' : 'Already in'} ${watchlist.name}.`,
        actionLabel: 'Open',
        onAction: () => openWatchlist(watchlist.id),
      });
    } catch (caught) {
      failed(caught);
    } finally {
      busy = false;
      choices = [];
    }
  }

  // Several lists ask where the video goes; Watch Later alone takes it.
  async function offer() {
    try {
      const watchlists = await api.youtubeWatchlists();
      if (watchlists.length > 1) choices = watchlists;
      else await save(watchlists[0]);
    } catch (caught) {
      failed(caught);
    }
  }

  onMount(() => {
    const shared = takeSharedYoutubeVideo();
    if (shared === null) return;
    if (!shared)
      showToast({
        key: 'youtube-share',
        tone: 'error',
        title: 'Nothing to save',
        message: 'Share a link to a YouTube video.',
      });
    else if (!available)
      showToast({
        key: 'youtube-share',
        tone: 'error',
        title: 'YouTube is unavailable',
        message: 'An administrator has turned YouTube off.',
      });
    else {
      videoId = shared;
      void offer();
    }
  });
</script>

{#if choices.length}
  <Modal title="Save to watchlist" {busy} onClose={() => (choices = [])}>
    <div class="grid gap-1">
      {#each choices as watchlist (watchlist.id)}
        {@const saved = watchlist.items.some(
          (item) => item.video.videoId === videoId,
        )}
        <button
          type="button"
          class="flex h-11 min-w-0 items-center gap-2 border border-line px-3 text-left text-sm enabled:hover:border-line-strong enabled:hover:bg-surface disabled:text-muted"
          disabled={busy || saved}
          onclick={() => void save(watchlist)}
        >
          <span class="min-w-0 flex-1 truncate">{watchlist.name}</span>
          {#if saved}
            <Check class="size-4 text-success" /><span class="sr-only"
              >Already saved</span
            >
          {:else}
            <span class="font-mono text-[0.6rem] text-muted"
              >{watchlist.items.length}</span
            >
          {/if}
        </button>
      {/each}
    </div>
  </Modal>
{/if}
