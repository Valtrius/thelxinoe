<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { untrack } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';

  let {
    id,
    kind,
  }: {
    id: string;
    kind: string;
  } = $props();
  type Flags = { favorite: boolean; watch_later: boolean; watched: boolean };
  let flags = $state<Flags | null>(null),
    error = $state(''),
    busy = $state(false),
    playlist = $state('');
  let playlists = $state<
      { id: string; name: string; can_add: boolean; contains?: boolean }[]
    >([]),
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
        const list = await api<{ items: typeof playlists }>(
          `/playlists?track=${encodeURIComponent(media)}`,
        );
        if (request === generation)
          playlists = list.items.filter((p) => p.can_add);
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
      await api(`/playlists/${playlist}/items`, 'POST', { items: [id] });
      playlists = playlists.map((p) =>
        p.id === playlist ? { ...p, contains: true } : p,
      );
      playlist = '';
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if flags}<div
    class="mt-3 mb-5 flex flex-wrap items-end gap-2.5 [&_label]:min-w-50"
  >
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      aria-pressed={flags.favorite}
      onclick={() => void toggle('favorite')}
      >{flags.favorite ? 'Remove favorite' : 'Favorite'}</Button
    >
    {#if ['movie', 'show', 'episode'].includes(kind)}<Button
        variant="secondary"
        size="form"
        disabled={busy}
        aria-pressed={flags.watch_later}
        onclick={() => void toggle('watch_later')}
        >{flags.watch_later ? 'Remove from Watch Later' : 'Watch Later'}</Button
      >{/if}
    {#if ['movie', 'episode', 'track'].includes(kind)}<Button
        variant="secondary"
        size="form"
        disabled={busy}
        aria-pressed={flags.watched}
        onclick={() => void toggle('watched')}
        >{flags.watched ? 'Mark unwatched' : 'Mark watched'}</Button
      >{/if}
    {#if kind === 'track' && playlists.length}<FormField
        >Playlist<select class={formControlClass} bind:value={playlist}
          ><option value="">Choose a playlist</option
          >{#each playlists as p (p.id)}<option
              value={p.id}
              disabled={p.contains}
              >{p.contains ? `In ${p.name}` : p.name}</option
            >{/each}</select
        ></FormField
      ><Button
        variant="secondary"
        size="form"
        disabled={busy || !playlist}
        onclick={() => void add()}>Add to playlist</Button
      >{/if}
  </div>{/if}
{#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
