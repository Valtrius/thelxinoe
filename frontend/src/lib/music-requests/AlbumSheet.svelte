<script lang="ts">
  import { onMount } from 'svelte';
  import { LibraryBig, Plus } from '@lucide/svelte';
  import { api } from '../api';
  import { length, plural } from '../music/library';
  import Button from '../ui/Button.svelte';
  import Modal from '../ui/Modal.svelte';
  import Notice from '../ui/Notice.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import AlbumCover from './AlbumCover.svelte';
  import {
    canRequest,
    requestable,
    requestLabel,
    requestMusic,
    scoped,
    type Album,
    type AlbumDetail,
    type MusicService,
    type RequestState,
  } from './types';
  let {
    album,
    service,
    close,
    openArtist,
    requested,
  }: {
    /** What the opening list knows, shown while the rest loads. */
    album: Album;
    service: MusicService;
    close: () => void;
    openArtist?: (id: string) => void;
    requested: (
      album: Album,
      request: { id: string; state: RequestState },
    ) => void;
  } = $props();
  let detail = $state<AlbumDetail | null>(null),
    error = $state(''),
    busy = $state(false);
  const shown = $derived<Album>(detail ?? album);
  const status = $derived(shown.request?.state);
  const discs = $derived(new Set(detail?.tracks.map((t) => t.disc)).size > 1);

  async function load() {
    error = '';
    try {
      detail = await api<AlbumDetail>(
        scoped(
          `/acquisition/music/albums/${encodeURIComponent(album.external_id)}`,
          service,
        ),
      );
    } catch (caught) {
      error = String(caught);
    }
  }
  async function request() {
    busy = true;
    error = '';
    try {
      const receipt = await requestMusic(service, album.external_id);
      if (detail) detail.request = receipt;
      requested(album, receipt);
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
  function released(date?: string | null) {
    if (!date) return '';
    if (date.length < 7) return date;
    return new Date(`${date.slice(0, 7)}-01T00:00:00Z`).toLocaleDateString(
      'en',
      { month: 'long', year: 'numeric', timeZone: 'UTC' },
    );
  }
  const clock = (seconds: number) =>
    `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
  onMount(() => void load());
</script>

<Modal title={shown.title} onClose={close}>
  <div class="flex flex-wrap items-start gap-5">
    <AlbumCover
      src={shown.cover}
      title={shown.title}
      artist={shown.artist}
      sizes="10rem"
      class="size-40 shrink-0 border border-line compact:size-28"
    />
    <div class="grid min-w-0 flex-[1_1_16rem] content-start gap-2">
      <p
        class="text-[10px] font-semibold tracking-[0.14em] text-accent uppercase"
      >
        {[shown.type ?? 'Album', released(shown.date) || shown.year]
          .filter(Boolean)
          .join(' · ')}
      </p>
      {#if shown.artist}{#if openArtist && shown.artist_id}<button
            type="button"
            class="justify-self-start text-sm font-semibold text-accent hover:underline"
            onclick={() => openArtist(shown.artist_id!)}>{shown.artist}</button
          >{:else}<span class="text-sm font-semibold">{shown.artist}</span
          >{/if}{/if}
      {#if detail}<p class="text-xs text-muted">
          {[
            detail.track_count ? plural(detail.track_count, 'track') : '',
            length(detail.duration),
            detail.label,
          ]
            .filter(Boolean)
            .join(' · ')}
        </p>
        {#if detail.edition || detail.format}<div
            class="flex flex-wrap gap-1.5"
          >
            {#each [detail.edition, detail.format].filter(Boolean) as chip (chip)}<span
                class="inline-flex h-5.5 items-center border border-line px-2 text-[10px] text-muted"
                >{chip}</span
              >{/each}
          </div>{/if}{/if}
      <div class="mt-1.5 flex flex-wrap items-center gap-2">
        {#if shown.in_library}<span
            class="inline-flex items-center gap-1.5 text-xs font-semibold text-accent"
            ><LibraryBig size={14} />In library</span
          >{:else if status && !requestable(status)}<span
            class={[
              'text-xs font-semibold',
              status === 'available' ? 'text-success' : 'text-muted',
            ]}>{requestLabel(status)}</span
          >{:else if shown.missing}<span
            class="text-xs font-semibold text-warning">Missing</span
          >{:else if canRequest(shown)}<Button
            size="form"
            loading={busy}
            onclick={() => void request()}
            ><Plus size={15} />Request album</Button
          >{:else if detail && detail.requestable === false}<span
            class="text-xs text-muted"
            >Lidarr's metadata profile leaves this release out.</span
          >{/if}
        {#if openArtist && shown.artist_id}<Button
            variant="secondary"
            size="form"
            onclick={() => openArtist(shown.artist_id!)}>Open artist</Button
          >{/if}
      </div>
    </div>
  </div>
  {#if error}<Notice variant="error" role="alert" class="mt-4">{error}</Notice
    >{/if}
  {#if !detail && !error}<div class="mt-6">
      <ContentSkeleton label="Loading album" />
    </div>{:else if detail?.tracks.length}<h3
      class="mt-6 mb-2 text-xs font-semibold text-muted"
    >
      Tracks
    </h3>
    <ol class="grid border-t border-line" aria-label="Tracks">
      {#each detail.tracks as track, index (`${track.disc}:${track.number}:${index}`)}{#if discs && track.number === 1}<li
            class="border-b border-line pt-3 pb-1.5 text-[10px] font-semibold tracking-widest text-muted uppercase"
          >
            Disc {track.disc}
          </li>{/if}
        <li
          class="grid grid-cols-[1.75rem_minmax(0,1fr)_auto] items-center gap-3 border-b border-line py-2 text-xs"
        >
          <span class="text-right text-muted tabular-nums">{track.number}</span
          ><span class="truncate">{track.title}</span><span
            class="text-muted tabular-nums"
            >{track.length ? clock(track.length) : ''}</span
          >
        </li>{/each}
    </ol>{/if}
</Modal>
