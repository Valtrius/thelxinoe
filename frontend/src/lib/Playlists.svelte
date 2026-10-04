<script lang="ts">
  import SectionHeading from './ui/SectionHeading.svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { onDestroy, untrack } from 'svelte';
  import { LatestRequest } from './latest-request';
  import { api, ApiError } from './api';
  import { captureSession } from './session';
  import { sessionPlaylistDrafts } from './playlist-drafts';
  import type { Card } from './media-state';
  import type { MediaChoice } from './playback';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
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
  let { userId, revision, play, selectedId, selectedChanged } = $props<{
    userId: string;
    revision: number;
    play: (item: MediaChoice) => void;
    selectedId?: string;
    selectedChanged: (id?: string, replace?: boolean) => void;
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
    creating = $state(false),
    baseRevision = $state(0);
  const drafts = sessionPlaylistDrafts();
  let loadingLists = $state(true);
  const ownsSession = captureSession();
  const editable = $derived(creating || selected?.owner_id === userId);
  const dirty = $derived(
    editable &&
      JSON.stringify([name, description, tracks.map((track) => track.id)]) !==
        JSON.stringify([
          selected?.name ?? '',
          selected?.description ?? '',
          selected?.items?.map((track) => track.id) ?? [],
        ]),
  );
  const conflict = $derived(selected && baseRevision !== selected.revision);
  function rememberDraft() {
    if (!ownsSession() || busy || (!selected && !creating)) return;
    if (dirty)
      drafts.set(selected?.id ?? 'new', {
        baseRevision,
        name,
        description,
        tracks: JSON.parse(JSON.stringify(tracks)),
      });
    else drafts.delete(selected?.id ?? 'new');
  }
  const lists = new LatestRequest(),
    selection = new LatestRequest(),
    searches = new LatestRequest();
  onDestroy(() => {
    rememberDraft();
    lists.invalidate();
    selection.invalidate();
    searches.invalidate();
  });
  $effect(() => {
    const id = selectedId;
    untrack(() => {
      rememberDraft();
      selection.invalidate();
      searches.invalidate();
      selected = null;
      baseRevision = 0;
      busy = false;
      creating = id === 'new';
      name = description = query = error = '';
      tracks = results = [];
      if (id && id !== 'new') void open(id);
      else if (id === 'new') restoreDraft(id);
    });
  });
  function restoreDraft(id: string) {
    const draft = drafts.get(id);
    if (draft) {
      baseRevision = draft.baseRevision;
      name = draft.name;
      description = draft.description;
      tracks = draft.tracks;
    }
  }
  function discardDraft() {
    drafts.delete(selected?.id ?? 'new');
    baseRevision = selected?.revision ?? 0;
    name = selected?.name ?? '';
    description = selected?.description ?? '';
    tracks = selected?.items ?? [];
  }
  $effect(() => {
    if (revision >= 0) untrack(() => void load());
  });
  async function load() {
    const current = lists.begin();
    loadingLists = true;
    try {
      const value = await api<{ items: Playlist[] }>('/playlists');
      if (current()) items = value.items;
    } catch (e) {
      if (current()) error = String(e);
    } finally {
      if (current()) loadingLists = false;
    }
  }
  async function open(id: string) {
    const current = selection.begin();
    searches.invalidate();
    busy = true;
    error = '';
    selected = null;
    creating = false;
    try {
      const value = await api<Playlist>(`/playlists/${id}`);
      if (!current()) return;
      selected = value;
      baseRevision = value.revision;
      creating = false;
      name = selected.name;
      description = selected.description;
      tracks = selected.items ?? [];
      restoreDraft(id);
      results = [];
    } catch (e) {
      if (current()) error = String(e);
    } finally {
      if (current()) busy = false;
    }
  }
  async function save(replace = false) {
    if (busy || !editable || (conflict && !replace)) return;
    const current = selection.begin();
    searches.invalidate();
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
          revision: replace ? (selected?.revision ?? 0) : baseRevision,
        },
      );
      if (!current()) return;
      drafts.delete(selected?.id ?? 'new');
      await load();
      if (!current()) return;
      if (creating) {
        creating = false;
        selectedChanged(value.id, true);
      } else await open(value.id);
    } catch (e) {
      if (current()) {
        if (e instanceof ApiError && e.status === 409 && selected) {
          try {
            const latest = await api<Playlist>(`/playlists/${selected.id}`);
            if (current()) {
              selected = latest;
              error = '';
            }
          } catch (caught) {
            if (current()) error = String(caught);
          }
        } else error = String(e);
      }
    } finally {
      if (current()) busy = false;
    }
  }
  async function remove() {
    if (!selected) return;
    const current = selection.begin();
    busy = true;
    error = '';
    try {
      await api(`/playlists/${selected.id}`, 'DELETE');
      if (!current()) return;
      drafts.delete(selected.id);
      selected = null;
      selectedChanged(undefined, true);
      await load();
    } catch (e) {
      if (current()) error = String(e);
    } finally {
      if (current()) busy = false;
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
    const current = searches.begin();
    try {
      const value = await api<{ items: Card[] }>(
        `/catalog?kind=track&q=${encodeURIComponent(query)}`,
      );
      if (current()) results = value.items;
    } catch (e) {
      if (current()) error = String(e);
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
</script>

<SectionHeading>
  <p class="text-muted">
    Everyone can play and favorite a playlist. Its owner controls the tracks.
  </p>
  <Button size="form" onclick={() => selectedChanged('new')}
    >New playlist</Button
  >
</SectionHeading>
{#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
{#if busy && !creating && !selected}<ContentSkeleton
    label="Loading playlist"
  />{/if}
{#if creating || selected}<Panel aria-label="Playlist details">
    <SectionHeading>
      <h2>{creating ? 'Create playlist' : selected?.name}</h2>
      <Button
        variant="secondary"
        size="form"
        onclick={() => selectedChanged(undefined)}>Close playlist</Button
      >
    </SectionHeading>
    <fieldset disabled={busy} class="min-w-0">
      {#if conflict && editable}<Notice tone="warning" role="alert">
          <p>
            This playlist changed elsewhere. Choose the saved version or
            explicitly replace it with your draft.
          </p>
          <details class="my-2">
            <summary>Saved playlist</summary>
            <p>{selected?.name}</p>
            {#if selected?.description}<p>{selected.description}</p>{/if}
            <ol class="list-inside list-decimal">
              {#each selected?.items ?? [] as track, index (index)}<li>
                  {track.title}
                </li>{/each}
            </ol>
          </details>
          <div class="flex flex-wrap gap-3">
            <Button variant="secondary" size="form" onclick={discardDraft}
              >Use saved playlist</Button
            >
            <Button variant="danger" size="form" onclick={() => void save(true)}
              >Replace with my draft</Button
            >
          </div>
        </Notice>{/if}
      {#if dirty}<div class="flex flex-wrap items-center gap-3">
          <p role="status">Unsaved playlist draft</p>
          <Button variant="secondary" size="form" onclick={discardDraft}
            >Discard changes</Button
          >
        </div>{/if}
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
          ><Button
            type="submit"
            size="form"
            disabled={busy || Boolean(conflict)}>Save playlist</Button
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
    </fieldset>
  </Panel>{/if}
{#if loadingLists && !items.length}<ContentSkeleton
    label="Loading playlists"
  />{:else}{#each items as item (item.id)}<Panel>
      <div class={rowClass}>
        <Button
          variant="secondary"
          size="form"
          onclick={() => selectedChanged(item.id)}>{item.name}</Button
        ><small>{item.owner} · {item.count} tracks</small><Button
          variant="secondary"
          size="form"
          aria-pressed={item.favorite}
          onclick={() => void favorite(item)}
          >{item.favorite ? 'Unfavorite playlist' : 'Favorite playlist'}</Button
        >
      </div>
    </Panel>{:else}{#if !error}<p class="text-muted">
        Create a playlist or save tracks from Music.
      </p>{/if}{/each}{/if}
