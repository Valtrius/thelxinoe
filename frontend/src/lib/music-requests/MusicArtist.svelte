<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    LibraryBig,
    Plus,
  } from '@lucide/svelte';
  import { api, type User } from '../api';
  import { plural } from '../music/library';
  import { showToast } from '../providers/toasts';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import AlbumCard from './AlbumCard.svelte';
  import AlbumSheet from './AlbumSheet.svelte';
  import ArtistPhoto from './ArtistPhoto.svelte';
  import MonitorButton from './MonitorButton.svelte';
  import {
    lidarr,
    markRequested,
    requestMusic,
    scoped,
    type Album,
    type ArtistView,
    type MusicService,
  } from './types';
  let { user, id, back, openLibrary } = $props<{
    user: User;
    /** The artist's MusicBrainz ID. */
    id: string;
    back: () => void;
    openLibrary: (item: string) => void;
  }>();
  let service = $state<MusicService | null>(null),
    view = $state<ArtistView | null>(null),
    error = $state(''),
    missingOnly = $state(false),
    newest = $state(true),
    picked = $state<string[]>([]),
    requesting = $state(false),
    othersOpen = $state(false),
    sheet = $state<Album | null>(null);
  let active = true;

  const held = $derived(
    view?.albums.filter((album) => album.in_library).length ?? 0,
  );
  const albums = $derived.by(() => {
    const list = (view?.albums ?? []).filter(
      (album) => !missingOnly || !album.in_library,
    );
    return newest ? list : [...list].reverse();
  });
  const otherKinds = $derived.by(() => {
    const counts: Record<string, number> = {};
    for (const album of view?.other ?? [])
      if (album.type && album.type !== 'Other')
        counts[album.type] = (counts[album.type] ?? 0) + 1;
    return Object.entries(counts)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 4)
      .map(([kind]) =>
        kind === 'EP'
          ? 'EPs'
          : kind === 'Live'
            ? 'live'
            : `${kind.toLowerCase()}s`,
      )
      .join(', ')
      .replace(/^./, (first) => first.toUpperCase());
  });

  async function load() {
    error = '';
    try {
      service = await lidarr();
      if (!active) return;
      if (!service?.ready) {
        error = 'Lidarr is not ready to request music.';
        return;
      }
      const result = await api<ArtistView>(
        scoped(`/acquisition/music/artists/${encodeURIComponent(id)}`, service),
      );
      if (active) view = result;
    } catch (caught) {
      if (active) error = String(caught);
    }
  }
  function toggle(album: Album) {
    picked = picked.includes(album.external_id)
      ? picked.filter((item) => item !== album.external_id)
      : [...picked, album.external_id];
  }
  async function requestPicked() {
    if (!service || !view) return;
    requesting = true;
    const failures: string[] = [];
    for (const album of view.albums.filter((album) =>
      picked.includes(album.external_id),
    )) {
      try {
        album.request = await requestMusic(service, album.external_id);
      } catch (caught) {
        failures.push(`${album.title}: ${caught}`);
      }
    }
    if (!active) return;
    requesting = false;
    picked = [];
    if (failures.length)
      showToast({
        key: `artist-request:${id}`,
        tone: 'error',
        title: view.artist.name,
        message: failures.join('\n'),
      });
  }
  onMount(() => void load());
  onDestroy(() => (active = false));
</script>

<div class="grid min-w-0 gap-6">
  <Button variant="ghost" size="sm" class="justify-self-start" onclick={back}
    ><ChevronLeft size={16} />Back to discover</Button
  >
  {#if error}<Notice variant="error" role="alert"
      >{error}<Button variant="ghost" size="sm" onclick={() => void load()}
        >Try again</Button
      ></Notice
    >{:else if !view || !service}<ContentSkeleton
      label="Loading artist"
      variant="detail"
    />{:else}
    <header
      class="flex flex-wrap items-center gap-6 compact:gap-4"
      aria-label={view.artist.name}
    >
      <ArtistPhoto src={view.artist.image} class="size-30 compact:size-23" />
      <div class="grid min-w-0 flex-[1_1_18rem] gap-2">
        <p
          class="text-[10px] font-semibold tracking-[0.14em] text-accent uppercase"
        >
          Artist · {held
            ? `${held} of ${plural(view.albums.length, 'album')} in library`
            : plural(view.albums.length, 'album')}
        </p>
        <h1
          class="text-4xl leading-tight font-semibold tracking-tight compact:text-3xl"
        >
          {view.artist.name}
        </h1>
        {#if view.artist.overview}<p
            class="line-clamp-3 max-w-[70ch] text-sm leading-6 text-muted"
          >
            {view.artist.overview}
          </p>{/if}
        {#if view.artist.type || view.artist.genres.length}<div
            class="flex flex-wrap gap-1.5"
          >
            {#each [view.artist.type, ...view.artist.genres].filter(Boolean) as chip (chip)}<span
                class="inline-flex h-5.5 items-center border border-line px-2 text-[10px] text-muted"
                >{chip}</span
              >{/each}
          </div>{/if}
        <div class="mt-2 flex flex-wrap gap-2">
          {#if view.artist.library_id}<Button
              variant="secondary"
              size="form"
              onclick={() => openLibrary(view!.artist.library_id!)}
              ><LibraryBig size={15} />Open in library</Button
            >{/if}
          <MonitorButton artist={view.artist} {service} {user} />
        </div>
      </div>
    </header>

    <section class="grid min-w-0 gap-4" aria-label="Albums">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <h2 class="flex items-baseline gap-2 text-base font-semibold">
          Albums<span class="text-xs font-normal text-muted"
            >{view.albums.length}</span
          >
        </h2>
        <div class="flex flex-wrap gap-3">
          {#if held}<div
              class="flex gap-1.5"
              role="group"
              aria-label="Album filter"
            >
              {#each [false, true] as only (only)}<button
                  type="button"
                  aria-pressed={missingOnly === only}
                  class={[
                    'h-7 border px-2.5 text-xs',
                    missingOnly === only
                      ? 'border-line-strong bg-accent-soft text-accent'
                      : 'border-line text-muted hover:text-foreground',
                  ]}
                  onclick={() => (missingOnly = only)}
                  >{only
                    ? `Not in library · ${view.albums.length - held}`
                    : 'All'}</button
                >{/each}
            </div>{/if}
          <div class="flex gap-1.5" role="group" aria-label="Album order">
            {#each [true, false] as order (order)}<button
                type="button"
                aria-pressed={newest === order}
                class={[
                  'h-7 border px-2.5 text-xs',
                  newest === order
                    ? 'border-line-strong bg-accent-soft text-accent'
                    : 'border-line text-muted hover:text-foreground',
                ]}
                onclick={() => (newest = order)}
                >{order ? 'Newest' : 'Oldest'}</button
              >{/each}
          </div>
        </div>
      </div>
      {#if albums.length}<div
          class="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-x-5 gap-y-7 compact:grid-cols-3 compact:gap-3"
        >
          {#each albums as album (album.external_id)}<AlbumCard
              {album}
              sizes="(max-width: 720px) 34vw, 12rem"
              byArtist={false}
              open={(album) => (sheet = album)}
              selected={picked.includes(album.external_id)}
              toggle={() => toggle(album)}
            />{/each}
        </div>{:else}<p class="py-6 text-sm text-muted">
          {view.albums.length
            ? 'Every album is in the library.'
            : `Lidarr's ${view.profile} metadata profile keeps none of this artist's releases.`}
        </p>{/if}
    </section>

    {#if picked.length}<div
        class="sticky bottom-3 z-10 flex flex-wrap items-center gap-2 border border-line-strong bg-surface-strong p-3 shadow-2xl compact:bottom-0"
      >
        <span class="flex-1 text-sm font-semibold"
          >{plural(picked.length, 'album')} selected</span
        >
        <Button
          variant="ghost"
          size="form"
          disabled={requesting}
          onclick={() => (picked = [])}>Clear</Button
        ><Button
          size="form"
          loading={requesting}
          onclick={() => void requestPicked()}
          ><Plus size={15} />Request {plural(picked.length, 'album')}</Button
        >
      </div>{/if}

    {#if view.other.length}<section
        class="border border-line bg-surface"
        aria-label="Other releases"
      >
        <button
          type="button"
          class="flex w-full items-center gap-3 px-3 py-2.5 text-left text-muted hover:text-foreground"
          aria-expanded={othersOpen}
          onclick={() => (othersOpen = !othersOpen)}
          >{#if othersOpen}<ChevronDown size={16} />{:else}<ChevronRight
              size={16}
            />{/if}<span class="flex-1 text-sm font-semibold text-foreground"
            >Other releases</span
          ><span class="text-xs"
            >{[otherKinds, view.other.length].filter(Boolean).join(' · ')}</span
          ></button
        >
        {#if othersOpen}<ul class="grid">
            {#each view.other as album (album.external_id)}<li>
                <button
                  type="button"
                  class="flex w-full items-center gap-3 border-t border-line px-3 py-2.5 text-left hover:bg-surface-soft"
                  onclick={() => (sheet = album)}
                  ><span class="min-w-0 flex-1 truncate text-xs font-semibold"
                    >{album.title}</span
                  ><span class="text-[11px] text-muted">{album.year ?? ''}</span
                  ><span
                    class="inline-flex h-5.5 items-center border border-line px-2 text-[10px] text-muted"
                    >{album.type}</span
                  ></button
                >
              </li>{/each}
            <li class="border-t border-line px-3 py-2.5 text-[11px] text-muted">
              Lidarr's {view.profile} metadata profile leaves these out.
            </li>
          </ul>{/if}
      </section>{/if}
  {/if}
</div>

{#if sheet && service}<AlbumSheet
    album={sheet}
    {service}
    close={() => (sheet = null)}
    requested={(album, request) => {
      if (view) markRequested([view.albums], album.external_id, request);
      picked = picked.filter((item) => item !== album.external_id);
    }}
  />{/if}
