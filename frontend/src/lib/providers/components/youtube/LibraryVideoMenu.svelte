<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import { Copy, Ellipsis, ExternalLink, Eye, RotateCcw } from '@lucide/svelte';
  import { copyText } from '../../../clipboard';
  import { api, normalizeError } from '../../api';
  import { dismissToastByKey, showToast } from '../../toasts';
  import type { YoutubeVideo } from '../../types';
  import { youtubeUrl } from '../../utils';
  import MenuBackdrop, {
    ignoreBackdrop,
  } from '../../../ui/MenuBackdrop.svelte';

  let { video, restart }: { video: YoutubeVideo; restart: () => void } =
    $props();
  let open = $state(false);

  const itemClass =
    'flex h-8 w-full cursor-default items-center gap-[0.55rem] whitespace-nowrap px-[0.55rem] py-[0.4rem] text-left text-[0.7rem] text-muted outline-none data-[highlighted]:bg-accent-soft data-[highlighted]:text-foreground [&_svg]:size-[0.85rem] [&_svg]:shrink-0 compact:h-11 compact:text-[13px]';

  // Takes the video's identity because Undo runs after the card is gone.
  function failed(videoId: string, title: string, caught: unknown) {
    showToast({
      key: `youtube-video:${videoId}`,
      tone: 'error',
      title,
      message: normalizeError(caught).message,
    });
  }

  async function attempt(action: () => Promise<unknown>) {
    const { videoId, title } = video;
    try {
      await action();
    } catch (caught) {
      failed(videoId, title, caught);
    }
  }

  async function markWatched() {
    const { videoId, title } = video;
    const key = `youtube-video:${videoId}`;
    try {
      await api.markYoutubeWatched(videoId);
    } catch (caught) {
      failed(videoId, title, caught);
      return;
    }
    // A watched video leaves Home, so Undo is where it is marked unwatched.
    showToast({
      key,
      tone: 'success',
      title,
      message: 'Marked watched.',
      actionLabel: 'Undo',
      onAction: () => {
        dismissToastByKey(key);
        api
          .markYoutubeUnwatched(videoId)
          .catch((caught) => failed(videoId, title, caught));
      },
    });
  }
</script>

<DropdownMenu.Root bind:open>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}<button
        {...props}
        class="grid size-7.5 min-h-0 place-items-center border border-[#ffffff30] bg-[#080b10c9] p-0 text-white"
        title="More actions"
        aria-label={`More actions for ${video.title}`}
        ><Ellipsis size={16} /></button
      >{/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    {#if open}<MenuBackdrop close={() => (open = false)} />{/if}
    <DropdownMenu.Content
      align="end"
      sideOffset={4}
      collisionPadding={8}
      preventScroll={false}
      onInteractOutside={ignoreBackdrop}
      class="z-50 grid min-w-52 border border-line-strong bg-surface-strong p-1.5 shadow-2xl"
      aria-label={`Actions for ${video.title}`}
    >
      <DropdownMenu.Item class={itemClass} onSelect={restart}
        ><RotateCcw />Play from beginning</DropdownMenu.Item
      >
      <DropdownMenu.Item class={itemClass} onSelect={() => void markWatched()}
        ><Eye />Mark watched</DropdownMenu.Item
      >
      <DropdownMenu.Separator class="my-1 h-px bg-line" />
      <DropdownMenu.Item
        class={itemClass}
        onSelect={() => void attempt(() => copyText(youtubeUrl(video.videoId)))}
        ><Copy />Copy YouTube URL</DropdownMenu.Item
      >
      <DropdownMenu.Item
        class={itemClass}
        onSelect={() =>
          void attempt(() => api.openExternal(youtubeUrl(video.videoId)))}
        ><ExternalLink />Open in browser</DropdownMenu.Item
      >
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>
