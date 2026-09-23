<script lang="ts">
  import SectionHeading from './ui/SectionHeading.svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { untrack } from 'svelte';
  import { api } from './api';
  import type { Card } from './media-state';
  import type { MediaChoice } from './playback';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { inlineFormClass, rowClass } from './ui/styles';
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

<SectionHeading>
  <p class="text-muted">
    Everyone can play and favorite a playlist. Its owner controls the tracks.
  </p>
  <Button
    size="form"
    onclick={() => {
      creating = true;
      selected = null;
      name = '';
      description = '';
      tracks = [];
      results = [];
    }}>New playlist</Button
  >
</SectionHeading>
{#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
{#if creating || selected}<Panel aria-label="Playlist details">
    <SectionHeading>
      <h2>{creating ? 'Create playlist' : selected?.name}</h2>
      <Button
        variant="secondary"
        size="form"
        onclick={() => {
          creating = false;
          selected = null;
        }}>Close playlist</Button
      >
    </SectionHeading>
    {#if editable}<form
        class="my-4 grid gap-3 [&_button]:justify-self-start"
        onsubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <FormField
          >Playlist name<input
            class={formControlClass}
            bind:value={name}
            required
            maxlength="160"
          /></FormField
        ><FormField
          >Description<textarea
            class={formControlClass}
            bind:value={description}
            maxlength="2000"></textarea></FormField
        ><Button type="submit" size="form" disabled={busy}>Save playlist</Button
        >
      </form>{:else}<p>{selected?.description}</p>
      <small>Owned by {selected?.owner}</small>{/if}
    <SectionHeading>
      <h3>{tracks.length} tracks</h3>
      <Button
        size="form"
        disabled={!tracks.some((t) => t.available)}
        onclick={() => start()}>Play playlist</Button
      >
    </SectionHeading>
    {#each tracks as track, index (index)}<div class={rowClass}>
        <span
          >{index + 1}. {track.title}{!track.available
            ? ' · Unavailable'
            : ''}</span
        >
        <div class="flex flex-wrap gap-2">
          <Button
            variant="secondary"
            size="form"
            disabled={!track.available}
            onclick={() => start(index)}>Play track {index + 1}</Button
          >{#if editable}<Button
              variant="secondary"
              size="form"
              aria-label={`Move track ${index + 1} up`}
              disabled={index === 0}
              onclick={() => move(index, -1)}>↑</Button
            ><Button
              variant="secondary"
              size="form"
              aria-label={`Move track ${index + 1} down`}
              disabled={index === tracks.length - 1}
              onclick={() => move(index, 1)}>↓</Button
            ><Button
              variant="secondary"
              size="form"
              onclick={() => (tracks = tracks.filter((_, i) => i !== index))}
              >Remove track {index + 1}</Button
            >{/if}
        </div>
      </div>{/each}
    {#if editable}<form
        class={inlineFormClass}
        onsubmit={(e) => {
          e.preventDefault();
          void search();
        }}
      >
        <FormField
          >Find tracks<input
            class={formControlClass}
            bind:value={query}
          /></FormField
        ><Button type="submit" variant="secondary" size="form"
          >Search tracks</Button
        >
      </form>
      {#each results as track (track.id)}<div class={rowClass}>
          <span>{track.title}</span><Button
            variant="secondary"
            size="form"
            disabled={tracks.length >= 500}
            onclick={() => (tracks = [...tracks, track])}
            >Add {track.title}</Button
          >
        </div>{/each}{/if}
    {#if selected && editable}<details class="mt-5">
        <summary>Delete playlist</summary>
        <p>Remove this playlist and its favorites for all users.</p>
        <Button
          variant="danger"
          size="form"
          disabled={busy}
          onclick={() => void remove()}>Delete this playlist</Button
        >
      </details>{/if}
  </Panel>{/if}
{#each items as item (item.id)}<Panel>
    <div class={rowClass}>
      <Button variant="secondary" size="form" onclick={() => void open(item.id)}
        >{item.name}</Button
      ><small>{item.owner} · {item.count} tracks</small><Button
        variant="secondary"
        size="form"
        aria-pressed={item.favorite}
        onclick={() => void favorite(item)}
        >{item.favorite ? 'Unfavorite playlist' : 'Favorite playlist'}</Button
      >
    </div>
  </Panel>{:else}<p class="text-muted">
    Create a playlist or save tracks from Music.
  </p>{/each}
