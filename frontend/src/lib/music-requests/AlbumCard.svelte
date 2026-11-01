<script lang="ts">
  import { Disc3, LibraryBig, Plus } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import { cover, requestable, requestLabel, type Album } from './types';
  let { album, request, busy, sizes } = $props<{
    album: Album;
    request: (album: Album) => void;
    busy: boolean;
    /** The cover's rendered width, for picking its image size. */
    sizes: string;
  }>();
  let failed = $state(false);
  const status = $derived(album.request?.state);
</script>

<article class="min-w-0 self-start" aria-label={album.title}>
  <div
    class="relative aspect-square overflow-hidden border border-line bg-surface-strong"
  >
    {#if cover(album.cover) && !failed}<img
        class="size-full object-cover"
        src={cover(album.cover)}
        {sizes}
        alt=""
        loading="lazy"
        onerror={() => (failed = true)}
      />{:else}<div class="grid size-full place-items-center p-4 text-muted">
        <Disc3 size={36} />
      </div>{/if}
    {#if album.in_library}<span
        class="absolute top-2 left-2 flex items-center gap-1 bg-black/80 px-1.5 py-1 text-[10px] text-white"
        data-in-library
        ><LibraryBig size={10} class="text-accent" />In library</span
      >{:else if status && !requestable(status)}<span
        class={[
          'absolute inset-x-0 bottom-0 px-2 py-2 text-center text-[10px] font-semibold backdrop-blur-md',
          status === 'available'
            ? 'bg-emerald-950/90 text-emerald-100'
            : 'bg-black/80 text-white',
        ]}>{requestLabel(status)}</span
      >{/if}
  </div>
  <strong class="mt-2.5 line-clamp-2 block text-xs leading-5 font-semibold"
    >{album.title}</strong
  >
  <span class="mt-0.5 block truncate text-[10px] text-muted"
    >{[album.artist, album.year].filter(Boolean).join(' · ')}</span
  >
  {#if !album.in_library && requestable(status)}<Button
      variant="secondary"
      size="sm"
      class="mt-2 w-full"
      loading={busy}
      disabled={busy}
      aria-label={`Request ${album.title}`}
      onclick={() => request(album)}><Plus size={14} />Request</Button
    >{/if}
</article>
