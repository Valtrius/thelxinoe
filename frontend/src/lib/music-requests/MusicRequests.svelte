<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { Bell, Disc3 } from '@lucide/svelte';
  import { api, type User } from '../api';
  import { plural } from '../music/library';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import AlbumCover from './AlbumCover.svelte';
  import ArtistPhoto from './ArtistPhoto.svelte';
  import {
    albumCover,
    artistImage,
    monitorLabel,
    requestLabel,
    type MusicRequest,
  } from './types';
  let { user, loaded } = $props<{
    user: User;
    loaded: (count: number) => void;
  }>();
  let requests = $state<MusicRequest[]>([]),
    busy = $state(false),
    error = $state('');
  let generation = 0,
    active = true;
  const admin = $derived(user.role === 'admin');

  // Pending requests one person made for one artist are decided together.
  const sorted = $derived.by(() => {
    const groups: Record<string, MusicRequest[]> = {};
    for (const request of requests)
      if (request.state === 'pending' && request.artist_id) {
        const key = `${request.user_id}:${request.artist_id}`;
        groups[key] = [...(groups[key] ?? []), request];
      }
    const grouped = Object.values(groups).filter((group) => group.length > 1);
    const inGroups = new Set(grouped.flat().map((request) => request.id));
    return {
      groups: grouped.map((group) =>
        // The artist's new releases lead its albums.
        [...group].sort(
          (a, b) =>
            Number(b.target === 'artist') - Number(a.target === 'artist'),
        ),
      ),
      single: requests.filter((request) => !inGroups.has(request.id)),
    };
  });

  export async function load() {
    const version = ++generation;
    try {
      const result = await api<{ items: MusicRequest[] }>(
        '/acquisition/requests',
      );
      if (!active || version !== generation) return;
      requests = result.items;
      error = '';
      loaded(requests.length);
    } catch (caught) {
      if (active && version === generation) error = String(caught);
    }
  }
  async function act(list: MusicRequest[], action: string) {
    busy = true;
    error = '';
    let failure = '';
    try {
      for (const request of list)
        await api(`/acquisition/requests/${request.id}`, 'POST', { action });
    } catch (caught) {
      failure = String(caught);
    }
    // The list shows what the decisions before a failure did.
    await load();
    if (failure) error = failure;
    busy = false;
  }
  const name = (request: MusicRequest) =>
    request.target === 'artist'
      ? (request.artist ?? request.title)
      : request.title;
  onMount(() => void load());
  onDestroy(() => {
    active = false;
  });
</script>

{#snippet actions(request: MusicRequest, quiet: boolean)}<div
    class="flex flex-wrap items-center gap-2"
  >
    {#if request.state === 'pending' && admin}<Button
        variant={quiet ? 'ghost' : 'default'}
        size="sm"
        disabled={busy}
        aria-label={`Approve ${name(request)}`}
        onclick={() => void act([request], 'approve')}>Approve</Button
      ><Button
        variant={quiet ? 'ghost' : 'secondary'}
        size="sm"
        disabled={busy}
        aria-label={`Decline ${name(request)}`}
        onclick={() => void act([request], 'deny')}>Decline</Button
      >{/if}
    {#if ['pending', 'failed', 'uncertain'].includes(request.state)}<Button
        variant="ghost"
        size="sm"
        disabled={busy}
        aria-label={`Cancel the request for ${name(request)}`}
        onclick={() => void act([request], 'cancel')}>Cancel</Button
      >{/if}
  </div>{/snippet}

{#snippet thumb(
  request: MusicRequest,
  size: string,
)}{#if request.target === 'artist'}<ArtistPhoto
      src={artistImage(request.artist_id ?? request.external_id)}
      class={size}
    />{:else if request.kind === 'lidarr'}<AlbumCover
      src={albumCover(request.external_id)}
      title={request.title}
      artist={request.artist}
      small
      sizes="3.25rem"
      class={`${size} shrink-0 border border-line`}
    />{:else}<span
      class={`grid ${size} shrink-0 place-items-center bg-surface-strong text-muted`}
      ><Disc3 size={20} /></span
    >{/if}{/snippet}

{#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
{#if requests.length}<section class="grid gap-3" aria-label="Music requests">
    <h3 class="mt-4 text-xs font-semibold text-muted">Music</h3>
    {#each sorted.groups as group (group[0].id)}{@const albums = group.filter(
        (request) => request.target !== 'artist',
      )}{@const releases = group.some((request) => request.target === 'artist')}
      <article
        class="border border-line-strong bg-surface"
        aria-label={`${group[0].artist} requests`}
      >
        <div class="flex flex-wrap items-center gap-4 p-4">
          <ArtistPhoto
            src={artistImage(group[0].artist_id!)}
            class="size-12"
          /><span class="grid min-w-0 flex-[1_1_14rem] gap-1"
            ><strong class="text-sm">{group[0].artist}</strong><span
              class="text-xs text-muted"
              >{[
                albums.length ? plural(albums.length, 'album') : '',
                releases ? 'new releases' : '',
              ]
                .filter(Boolean)
                .join(' and ')} · Pending approval</span
            >{#if admin}<span class="text-[10px] text-muted"
                >{group[0].username}</span
              >{/if}</span
          >
          {#if admin}<div class="flex flex-wrap gap-2">
              <Button
                size="sm"
                disabled={busy}
                onclick={() => void act(group, 'approve')}>Approve all</Button
              ><Button
                variant="secondary"
                size="sm"
                disabled={busy}
                onclick={() => void act(group, 'deny')}>Decline all</Button
              >
            </div>{/if}
        </div>
        {#each group as request (request.id)}<div
            class="flex flex-wrap items-center gap-3.5 border-t border-line py-2.5 pr-4 pl-20 compact:pl-4"
          >
            {#if request.target === 'artist'}<span
                class="grid size-10 shrink-0 place-items-center border border-line text-accent"
                ><Bell size={17} /></span
              ><span class="min-w-0 flex-[1_1_12rem] text-[13px] font-semibold"
                >Monitor new releases</span
              >{:else}{@render thumb(request, 'size-10')}<span
                class="min-w-0 flex-[1_1_12rem] truncate text-[13px] font-semibold"
                >{request.title}</span
              >{/if}
            {@render actions(request, true)}
          </div>{/each}
      </article>{/each}
    {#each sorted.single as request (request.id)}<article
        class="flex items-center gap-4 border border-line bg-surface p-4 compact:flex-wrap"
        aria-label={name(request)}
      >
        <div class="flex min-w-0 flex-1 items-center gap-4">
          {@render thumb(request, 'size-13')}<span class="min-w-0"
            ><strong class="block text-sm">{name(request)}</strong><span
              class="mt-1 block text-xs text-muted"
              >{#if request.target === 'artist'}{monitorLabel(
                  request.state,
                )}{:else}{request.artist ? `${request.artist} · ` : ''}<span
                  class={request.state === 'available' ? 'text-success' : ''}
                  >{requestLabel(request.state)}</span
                >{/if}</span
            >{#if request.error && ['failed', 'uncertain'].includes(request.state)}<span
                class="mt-1 block text-xs text-danger">{request.error}</span
              >{/if}{#if admin}<span class="mt-1 block text-[10px] text-muted"
                >{request.username}</span
              >{/if}</span
          >
        </div>
        {@render actions(request, false)}
      </article>{/each}
  </section>{/if}
