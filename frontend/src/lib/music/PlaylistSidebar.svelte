<script lang="ts">
  import { ArrowDown, ArrowUp } from '@lucide/svelte';
  import ListSidebar from '../ui/ListSidebar.svelte';
  import Switch from '../ui/Switch.svelte';
  import { eyebrowTextClass } from '../ui/styles';
  import PlaylistItem from './PlaylistItem.svelte';
  import type { PlaylistSort, PlaylistTrack } from './playlist-order';
  import { playlists, type Playlist } from './playlists.svelte';

  let {
    play,
    details,
    openAlbum,
    openArtist,
    onSelect,
    onClose,
  }: {
    play: (track: PlaylistTrack) => void;
    details: (track: PlaylistTrack) => void;
    openAlbum: (track: PlaylistTrack) => void;
    openArtist: (track: PlaylistTrack) => void;
    onSelect: (id: string) => void;
    onClose?: () => void;
  } = $props();

  const selected = $derived(playlists.selected);
  const sorts: [Exclude<PlaylistSort, 'manual'>, string][] = [
    ['added', 'Date added'],
    ['title', 'Title'],
    ['artist', 'Artist'],
  ];
  let renameValue = $derived(selected?.name ?? '');
  let renameError = $state('');

  async function rename(list: Playlist) {
    const name = renameValue.trim();
    if (!name || name === list.name) {
      renameValue = list.name;
      renameError = '';
      return;
    }
    renameError = (await playlists.update(list.id, { name })) ?? '';
  }
  const update = (
    list: Playlist,
    change: Parameters<typeof playlists.update>[1],
  ) => void playlists.update(list.id, change);
</script>

{#snippet sort(list: Playlist)}
  <p class={`${eyebrowTextClass} mb-2`}>SORT BY</p>
  <div class="flex items-center gap-1 py-1.5">
    <span class="flex-1 text-sm">Manual order</span>
    <button
      type="button"
      class={`h-8 border px-2.5 text-xs ${list.sort_mode === 'manual' ? 'border-line-strong bg-accent-soft text-accent' : 'border-line text-muted hover:text-foreground'}`}
      aria-pressed={list.sort_mode === 'manual'}
      onclick={() => update(list, { sort_mode: 'manual' })}>Manual</button
    >
  </div>
  {#each sorts as [mode, label] (mode)}
    <div class="flex items-center gap-1 border-t border-line py-1.5">
      <span class="flex-1 text-sm">{label}</span>
      {#each ['asc', 'desc'] as const as direction (direction)}
        {@const active =
          list.sort_mode === mode && list.sort_direction === direction}
        <button
          type="button"
          class={`grid size-8 place-items-center border ${active ? 'border-line-strong bg-accent-soft text-accent' : 'border-line text-muted hover:text-foreground'}`}
          aria-label={`${label} ${direction === 'asc' ? 'ascending' : 'descending'}`}
          aria-pressed={active}
          onclick={() =>
            update(list, { sort_mode: mode, sort_direction: direction })}
          >{#if direction === 'asc'}<ArrowUp
              class="size-3.5"
            />{:else}<ArrowDown class="size-3.5" />{/if}</button
        >
      {/each}
    </div>
  {/each}
{/snippet}

{#snippet settings(list: Playlist)}
  <p class={`${eyebrowTextClass} mb-2`}>PLAYLIST SETTINGS</p>
  <label class="mb-2 block">
    <span class="mb-1 block text-muted">Name</span>
    <input
      class="h-8 w-full border border-line bg-background px-2 outline-none focus:border-line-strong"
      maxlength="160"
      bind:value={renameValue}
      oninput={() => (renameError = '')}
      onkeydown={(event) => {
        if (event.key === 'Enter') void rename(list);
      }}
      onblur={() => void rename(list)}
    />
    {#if renameError}<span class="mt-1 block text-danger" role="alert"
        >{renameError}</span
      >{/if}
  </label>
  <div class="border-t border-line py-2">
    <Switch
      size="sm"
      checked={list.access !== 'private'}
      onCheckedChange={(on) =>
        update(list, { access: on ? 'read' : 'private' })}
    >
      Public
    </Switch>
  </div>
  <div class="border-t border-line py-2">
    <Switch
      size="sm"
      disabled={list.access === 'private'}
      checked={list.access === 'add' || list.access === 'edit'}
      onCheckedChange={(on) => update(list, { access: on ? 'add' : 'read' })}
    >
      Others can add
    </Switch>
  </div>
  <div class="border-t border-line py-2">
    <Switch
      size="sm"
      disabled={list.access !== 'add' && list.access !== 'edit'}
      checked={list.access === 'edit'}
      onCheckedChange={(on) => update(list, { access: on ? 'edit' : 'add' })}
    >
      Others can remove and reorder
    </Switch>
  </div>
{/snippet}

<ListSidebar
  noun="playlist"
  lists={playlists.lists}
  {selected}
  items={playlists.shown}
  fallbackName="Playlists"
  emptyText={playlists.ready ? 'Add tracks from the ⋯ menu on any track.' : ''}
  itemKey={(track) => track.id}
  canReorder={playlists.canReorder}
  rowHeight={53}
  canDelete={(list) => list.is_owner}
  sortMenu={selected?.can_edit ? sort : undefined}
  settingsMenu={selected?.is_owner ? settings : undefined}
  {onSelect}
  onCreate={(name) => playlists.create(name)}
  onDelete={(id) => playlists.remove(id)}
  onReorder={(id, order, moved) => playlists.moveTrack(id, order, moved)}
  {onClose}
>
  {#snippet listCaption(list)}
    {#if !list.is_owner}<span
        class="ml-2 max-w-24 shrink-0 truncate text-[0.62rem] text-muted"
        >{list.owner}</span
      >{/if}
  {/snippet}
  {#snippet row(track, { sortable, dragging, suppressClick, onReorderStart })}
    <PlaylistItem
      {track}
      canRemove={!!selected?.can_edit}
      {sortable}
      {dragging}
      {suppressClick}
      {onReorderStart}
      play={() => play(track)}
      remove={() =>
        selected && void playlists.removeTrack(selected.id, track.id)}
      details={() => details(track)}
      openAlbum={track.parent_id ? () => openAlbum(track) : undefined}
      openArtist={track.artist_id ? () => openArtist(track) : undefined}
    />
  {/snippet}
</ListSidebar>
