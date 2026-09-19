<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from './api';
  import type { Card } from './media-state';
  import type { MediaChoice } from './playback';
  type Playlist = {
    id: string;
    name: string;
    description: string;
    owner_id: string;
    owner: string;
    favorite: boolean;
    count: number;
    revision: number;
    items?: Card[];
  };
  let { userId, revision, play } = $props<{
    userId: string;
    revision: number;
    play: (item: MediaChoice) => void;
  }>();
  let items = $state<Playlist[]>([]),
    selected = $state<Playlist | null>(null),
    name = $state(''),
    description = $state(''),
    tracks = $state<Card[]>([]),
    query = $state(''),
    results = $state<Card[]>([]),
    error = $state(''),
    busy = $state(false),
    creating = $state(false);
  $effect(() => {
    if (revision >= 0) untrack(() => void load());
  });
  async function load() {
    try {
      items = (await api<{ items: Playlist[] }>('/playlists')).items;
    } catch (e) {
      error = String(e);
    }
  }
  async function open(id: string) {
    busy = true;
    error = '';
    try {
      selected = await api<Playlist>(`/playlists/${id}`);
      creating = false;
      name = selected.name;
      description = selected.description;
      tracks = selected.items ?? [];
      results = [];
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function save() {
    busy = true;
    error = '';
    try {
      const value = await api<{ id: string }>(
        creating ? '/playlists' : `/playlists/${selected!.id}`,
        creating ? 'POST' : 'PUT',
        {
          name,
          description,
          items: tracks.map((t) => t.id),
          revision: selected?.revision ?? 0,
        },
      );
      await load();
      await open(value.id);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function remove() {
    if (!selected) return;
    busy = true;
    error = '';
    try {
      await api(`/playlists/${selected.id}`, 'DELETE');
      selected = null;
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function favorite(p: Playlist) {
    try {
      await api(`/playlists/${p.id}/favorite`, 'PUT', {
        favorite: !p.favorite,
      });
      await load();
      if (selected?.id === p.id) selected.favorite = !p.favorite;
    } catch (e) {
      error = String(e);
    }
  }
  async function search() {
    try {
      results = (
        await api<{ items: Card[] }>(
          `/catalog?kind=track&q=${encodeURIComponent(query)}`,
        )
      ).items;
    } catch (e) {
      error = String(e);
    }
  }
  function move(index: number, delta: number) {
    const next = [...tracks];
    [next[index], next[index + delta]] = [next[index + delta], next[index]];
    tracks = next;
  }
  function start(index = 0) {
    const available = tracks
      .map((t, i) => ({ ...t, original: i }))
      .filter((t) => t.available);
    const selectedIndex = available.findIndex((t) => t.original >= index);
    const track = available[selectedIndex];
    if (track) play({ ...track, queueIndex: selectedIndex, queue: available });
  }
  const editable = $derived(creating || selected?.owner_id === userId);
</script>

<div class="section-heading">
  <p class="muted">
    Everyone can play and favorite a playlist. Its owner controls the tracks.
  </p>
  <button
    class="primary"
    onclick={() => {
      creating = true;
      selected = null;
      name = '';
      description = '';
      tracks = [];
      results = [];
    }}>New playlist</button
  >
</div>
{#if error}<p role="alert" class="error">{error}</p>{/if}
{#if creating || selected}<section class="panel" aria-label="Playlist details">
    <div class="section-heading">
      <h2>{creating ? 'Create playlist' : selected?.name}</h2>
      <button
        class="secondary"
        onclick={() => {
          creating = false;
          selected = null;
        }}>Close playlist</button
      >
    </div>
    {#if editable}<form
        onsubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <label
          >Playlist name<input
            bind:value={name}
            required
            maxlength="160"
          /></label
        ><label
          >Description<textarea bind:value={description} maxlength="2000"
          ></textarea></label
        ><button class="primary" disabled={busy}>Save playlist</button>
      </form>{:else}<p>{selected?.description}</p>
      <small>Owned by {selected?.owner}</small>{/if}
    <div class="section-heading">
      <h3>{tracks.length} tracks</h3>
      <button
        class="primary"
        disabled={!tracks.some((t) => t.available)}
        onclick={() => start()}>Play playlist</button
      >
    </div>
    {#each tracks as track, index (index)}<div class="row">
        <span
          >{index + 1}. {track.title}{!track.available
            ? ' · Unavailable'
            : ''}</span
        >
        <div class="track-actions">
          <button
            class="secondary"
            disabled={!track.available}
            onclick={() => start(index)}>Play track {index + 1}</button
          >{#if editable}<button
              class="secondary"
              aria-label={`Move track ${index + 1} up`}
              disabled={index === 0}
              onclick={() => move(index, -1)}>↑</button
            ><button
              class="secondary"
              aria-label={`Move track ${index + 1} down`}
              disabled={index === tracks.length - 1}
              onclick={() => move(index, 1)}>↓</button
            ><button
              class="secondary"
              onclick={() => (tracks = tracks.filter((_, i) => i !== index))}
              >Remove track {index + 1}</button
            >{/if}
        </div>
      </div>{/each}
    {#if editable}<form
        class="inline-form"
        onsubmit={(e) => {
          e.preventDefault();
          void search();
        }}
      >
        <label>Find tracks<input bind:value={query} /></label><button
          class="secondary">Search tracks</button
        >
      </form>
      {#each results as track (track.id)}<div class="row">
          <span>{track.title}</span><button
            class="secondary"
            disabled={tracks.length >= 500}
            onclick={() => (tracks = [...tracks, track])}
            >Add {track.title}</button
          >
        </div>{/each}{/if}
    {#if selected && editable}<details>
        <summary>Delete playlist</summary>
        <p>Remove this playlist and its favorites for all users.</p>
        <button class="danger" disabled={busy} onclick={() => void remove()}
          >Delete this playlist</button
        >
      </details>{/if}
  </section>{/if}
{#each items as item (item.id)}<section class="panel">
    <div class="row">
      <button class="secondary" onclick={() => void open(item.id)}
        >{item.name}</button
      ><small>{item.owner} · {item.count} tracks</small><button
        class="secondary"
        aria-pressed={item.favorite}
        onclick={() => void favorite(item)}
        >{item.favorite ? 'Unfavorite playlist' : 'Favorite playlist'}</button
      >
    </div>
  </section>{:else}<p class="muted">
    Create a playlist or save tracks from Music.
  </p>{/each}

<style>
  .track-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  form {
    display: grid;
    gap: 12px;
    margin: 16px 0;
  }
  form button {
    justify-self: start;
  }
  details {
    margin-top: 20px;
  }
</style>
