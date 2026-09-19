<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from './api';
  let { id, kind, userId } = $props<{
    id: string;
    kind: string;
    userId: string;
  }>();
  type Flags = { favorite: boolean; watch_later: boolean; watched: boolean };
  let flags = $state<Flags | null>(null),
    error = $state(''),
    busy = $state(false),
    playlist = $state('');
  let playlists = $state<{ id: string; name: string; owner_id: string }[]>([]),
    generation = 0;
  $effect(() => {
    const media = id,
      type = kind;
    untrack(() => void load(media, type));
  });
  async function load(media: string, type: string) {
    const request = ++generation;
    flags = null;
    error = '';
    playlist = '';
    playlists = [];
    try {
      const result = await api<Flags>(`/catalog/${media}/state`);
      if (request !== generation) return;
      flags = result;
      if (type === 'track') {
        const list = await api<{ items: typeof playlists }>('/playlists');
        if (request === generation)
          playlists = list.items.filter((p) => p.owner_id === userId);
      }
    } catch (e) {
      if (request === generation) error = String(e);
    }
  }
  async function toggle(key: keyof Flags) {
    if (!flags) return;
    busy = true;
    error = '';
    const media = id,
      request = generation;
    try {
      const result = await api<Flags>(`/catalog/${media}/state`, 'PUT', {
        [key]: !flags[key],
      });
      if (request === generation) flags = result;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function add() {
    busy = true;
    error = '';
    try {
      const value = await api<{
        name: string;
        description: string;
        revision: number;
        items: { id: string }[];
      }>(`/playlists/${playlist}`);
      await api(`/playlists/${playlist}`, 'PUT', {
        ...value,
        items: [...value.items.map((i) => i.id), id],
      });
      playlist = '';
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if flags}<div class="actions">
    <button
      class="secondary"
      disabled={busy}
      aria-pressed={flags.favorite}
      onclick={() => void toggle('favorite')}
      >{flags.favorite ? 'Remove favorite' : 'Favorite'}</button
    >
    {#if ['movie', 'show', 'episode'].includes(kind)}<button
        class="secondary"
        disabled={busy}
        aria-pressed={flags.watch_later}
        onclick={() => void toggle('watch_later')}
        >{flags.watch_later ? 'Remove from Watch Later' : 'Watch Later'}</button
      >{/if}
    {#if ['movie', 'episode', 'track'].includes(kind)}<button
        class="secondary"
        disabled={busy}
        aria-pressed={flags.watched}
        onclick={() => void toggle('watched')}
        >{flags.watched ? 'Mark unwatched' : 'Mark watched'}</button
      >{/if}
    {#if kind === 'track' && playlists.length}<label
        >Playlist<select bind:value={playlist}
          ><option value="">Choose your playlist</option
          >{#each playlists as p (p.id)}<option value={p.id}>{p.name}</option
            >{/each}</select
        ></label
      ><button
        class="secondary"
        disabled={busy || !playlist}
        onclick={() => void add()}>Add to playlist</button
      >{/if}
  </div>{/if}
{#if error}<p class="error" role="alert">{error}</p>{/if}

<style>
  .actions {
    display: flex;
    align-items: end;
    gap: 10px;
    flex-wrap: wrap;
    margin: 12px 0 20px;
  }
  .actions label {
    min-width: 200px;
  }
</style>
