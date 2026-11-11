<script lang="ts">
  import { Check, LibraryBig, Plus } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import { plural } from '../music/library';
  import AlbumCover from './AlbumCover.svelte';
  import { canRequest, requestable, requestLabel, type Album } from './types';
  let {
    album,
    sizes,
    open,
    openArtist,
    byArtist = true,
    request,
    busy = false,
    selected = false,
    toggle,
  }: {
    album: Album;
    /** The cover's rendered width, for picking its image size. */
    sizes: string;
    open: (album: Album) => void;
    openArtist?: (id: string) => void;
    /** Name the artist under the title; an artist's own page names the tracks instead. */
    byArtist?: boolean;
    request?: (album: Album) => void;
    busy?: boolean;
    selected?: boolean;
    /** Picks the album for a request of several. */
    toggle?: () => void;
  } = $props();
  const status = $derived(album.request?.state);
  const show = () => open(album);
</script>

<article class="min-w-0 self-start" aria-label={album.title}>
  <div
    class={[
      'relative aspect-square overflow-hidden border bg-surface-strong',
      selected
        ? 'border-accent outline-2 -outline-offset-2 outline-accent'
        : 'border-line',
    ]}
  >
    <button
      type="button"
      class="block size-full text-left"
      tabindex="-1"
      aria-hidden="true"
      onclick={show}
    >
      <AlbumCover
        src={album.cover}
        title={album.title}
        artist={album.artist}
        {sizes}
        class="size-full"
      />
    </button>
    {#if album.in_library}<span
        class="pointer-events-none absolute top-2 left-2 flex items-center gap-1 bg-black/80 px-1.5 py-1 text-[10px] text-white"
        data-in-library
        ><LibraryBig size={10} class="text-accent" />In library</span
      >{:else if status && !requestable(status)}<span
        class={[
          'pointer-events-none absolute inset-x-0 bottom-0 px-2 py-2 text-center text-[10px] font-semibold backdrop-blur-md',
          status === 'available'
            ? 'bg-emerald-950/90 text-emerald-100'
            : 'bg-black/80 text-white',
        ]}>{requestLabel(status)}</span
      >{:else if album.missing}<span
        class="pointer-events-none absolute inset-x-0 bottom-0 bg-black/85 px-2 py-2 text-center text-[10px] font-semibold text-warning backdrop-blur-md"
        >Missing</span
      >{/if}
    {#if toggle && canRequest(album)}<button
        type="button"
        class="absolute top-0 right-0 grid size-11 place-items-center"
        aria-pressed={selected}
        aria-label={`${selected ? 'Deselect' : 'Select'} ${album.title}`}
        onclick={toggle}
        ><span
          class={[
            'grid size-5.5 place-items-center border',
            selected
              ? 'border-accent bg-accent text-background'
              : 'border-white/60 bg-black/55 text-transparent',
          ]}><Check size={13} strokeWidth={3} /></span
        ></button
      >{/if}
  </div>
  <button
    type="button"
    class="mt-2.5 line-clamp-2 block text-left text-xs leading-5 font-semibold hover:text-accent"
    onclick={show}>{album.title}</button
  >
  <span class="mt-0.5 block truncate text-[10px] text-muted"
    >{#if byArtist}{#if album.artist && album.artist_id && openArtist}<button
          type="button"
          class="hover:text-foreground hover:underline"
          onclick={() => openArtist(album.artist_id!)}>{album.artist}</button
        >{:else}{album.artist ?? ''}{/if}{album.artist && album.year
        ? ' · '
        : ''}{album.year ?? ''}{:else}{[
        album.year,
        album.type && album.type !== 'Album' ? album.type : '',
        album.track_count ? plural(album.track_count, 'track') : '',
      ]
        .filter(Boolean)
        .join(' · ')}{/if}</span
  >
  {#if request && !toggle && canRequest(album)}<Button
      variant="secondary"
      size="sm"
      class="mt-2 w-full"
      loading={busy}
      disabled={busy}
      aria-label={`Request ${album.title}`}
      onclick={() => request(album)}><Plus size={14} />Request</Button
    >{/if}
</article>
