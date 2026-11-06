<script lang="ts">
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import { ArrowDown, ArrowUp, LoaderCircle } from '@lucide/svelte';
  import type {
    YoutubeDownloadEvent,
    YoutubeVideo,
    YoutubeWatchlist,
    YoutubeWatchlistUpdate,
  } from '../../types';
  import { sortedYoutubeWatchlistItems } from '../../youtube-watchlists';
  import ListSidebar from '../../../ui/ListSidebar.svelte';
  import Switch from '../../../ui/Switch.svelte';
  import { eyebrowTextClass as eyebrowClass } from '../../../ui/styles';
  import WatchlistItem from './WatchlistItem.svelte';

  let {
    actions,
    watchlists,
    selectedWatchlistId,
    downloadProgress,
    downloadBusyIds,
    activeVideoIds,
    launchingVideoIds,
    playerReady,
    downloadsEnabled,
    onSelect,
    onCreate,
    onUpdate,
    onDelete,
    onReorder,
    onRemove,
    onClose,
  }: {
    actions: YoutubeVideoActions;
    watchlists: YoutubeWatchlist[];
    selectedWatchlistId: number | null;
    downloadProgress: Map<string, YoutubeDownloadEvent>;
    downloadBusyIds: Set<string>;
    activeVideoIds: Set<string>;
    launchingVideoIds: Set<string>;
    playerReady: boolean;
    downloadsEnabled: boolean;
    onSelect: (watchlistId: number) => void;
    onCreate: (name: string) => Promise<void>;
    onUpdate: (
      watchlistId: number,
      patch: Partial<YoutubeWatchlistUpdate>,
    ) => Promise<void>;
    onDelete: (watchlistId: number) => Promise<void>;
    onReorder: (watchlistId: number, videoIds: string[]) => Promise<void>;
    onRemove: (watchlistId: number, video: YoutubeVideo) => void;
    onClose?: () => void;
  } = $props();

  const lists = $derived(
    watchlists.map((watchlist) => ({
      ...watchlist,
      count: watchlist.items.length,
    })),
  );
  const selected = $derived(
    lists.find((watchlist) => watchlist.id === selectedWatchlistId) ?? lists[0],
  );
  let renameValue = $derived(selected?.name ?? '');
  const canReorder = $derived(
    selected?.sortMode === 'manual' &&
      !selected.items.some((item) => item.metadataPending),
  );
  const items = $derived(selected ? sortedYoutubeWatchlistItems(selected) : []);

  async function updateSelected(patch: Partial<YoutubeWatchlistUpdate>) {
    if (!selected) return;
    await onUpdate(selected.id, patch);
  }
</script>

<ListSidebar
  noun="watchlist"
  {lists}
  {selected}
  {items}
  fallbackName="Watch Later"
  emptyText="Add videos from the ⋯ menu on any YouTube card."
  itemKey={(item) => item.video.videoId}
  {canReorder}
  canDelete={(watchlist) => !watchlist.isDefault}
  {onSelect}
  {onCreate}
  {onDelete}
  {onReorder}
  {onClose}
>
  {#snippet sortMenu(watchlist)}
    <p class={`${eyebrowClass} mb-2`}>SORT BY</p>
    <div class="flex items-center gap-1 py-1.5">
      <span class="flex-1 text-sm">Manual order</span>
      <button
        type="button"
        class={`h-8 border px-2.5 text-xs ${watchlist.sortMode === 'manual' ? 'border-line-strong bg-accent-soft text-accent' : 'border-line text-muted hover:text-foreground'}`}
        aria-pressed={watchlist.sortMode === 'manual'}
        onclick={() => void updateSelected({ sortMode: 'manual' })}
        >Manual</button
      >
    </div>
    <div class="flex items-center gap-1 border-t border-line py-1.5">
      <span class="flex-1 text-sm">Date</span>
      <button
        type="button"
        class={`grid size-8 place-items-center border ${watchlist.sortMode === 'date' && watchlist.sortDirection === 'asc' ? 'border-line-strong bg-accent-soft text-accent' : 'border-line text-muted hover:text-foreground'}`}
        aria-label="Date ascending"
        aria-pressed={watchlist.sortMode === 'date' &&
          watchlist.sortDirection === 'asc'}
        onclick={() =>
          void updateSelected({
            sortMode: 'date',
            sortDirection: 'asc',
          })}><ArrowUp class="size-3.5" /></button
      >
      <button
        type="button"
        class={`grid size-8 place-items-center border ${watchlist.sortMode === 'date' && watchlist.sortDirection === 'desc' ? 'border-line-strong bg-accent-soft text-accent' : 'border-line text-muted hover:text-foreground'}`}
        aria-label="Date descending"
        aria-pressed={watchlist.sortMode === 'date' &&
          watchlist.sortDirection === 'desc'}
        onclick={() =>
          void updateSelected({
            sortMode: 'date',
            sortDirection: 'desc',
          })}><ArrowDown class="size-3.5" /></button
      >
    </div>
  {/snippet}
  {#snippet settingsMenu(watchlist)}
    <p class={`${eyebrowClass} mb-2`}>WATCHLIST SETTINGS</p>
    {#if !watchlist.isDefault}
      <label class="mb-2 block">
        <span class="mb-1 block text-muted">Name</span>
        <input
          class="h-8 w-full border border-line bg-background px-2 outline-none focus:border-line-strong"
          maxlength="80"
          bind:value={renameValue}
          onkeydown={(event) => {
            if (
              event.key === 'Enter' &&
              renameValue.trim() &&
              renameValue.trim() !== watchlist.name
            )
              void updateSelected({ name: renameValue.trim() });
          }}
          onblur={() => {
            if (renameValue.trim() && renameValue.trim() !== watchlist.name)
              void updateSelected({ name: renameValue.trim() });
            else renameValue = watchlist.name;
          }}
        />
      </label>
    {/if}
    <Switch
      size="sm"
      class="flex border-t border-line py-2"
      checked={watchlist.autoDownload}
      onCheckedChange={(checked) =>
        void updateSelected({ autoDownload: checked })}
    >
      Automatically download added videos
    </Switch>
    <Switch
      size="sm"
      class="flex border-t border-line py-2"
      checked={watchlist.autoRemoveWatched}
      onCheckedChange={(checked) =>
        void updateSelected({ autoRemoveWatched: checked })}
    >
      Remove videos when watched
    </Switch>
  {/snippet}
  {#snippet row(item, { sortable, dragging, suppressClick, onReorderStart })}
    {#if item.metadataPending}
      <div class="flex items-stretch border-b border-line" aria-busy="true">
        <div
          class="grid aspect-video w-[min(10rem,42%)] shrink-0 place-items-center bg-[linear-gradient(130deg,#121923,#080b10_60%)]"
        >
          <LoaderCircle class="size-5 animate-spin text-accent" />
        </div>
        <div class="min-w-0 flex-1 px-2.5 py-1">
          <p class="text-sm leading-5 font-medium">Loading video…</p>
          <p class="mt-0.5 truncate text-[0.62rem] text-muted">
            {item.video.videoId}
          </p>
        </div>
      </div>
    {:else}
      <WatchlistItem
        {actions}
        video={item.video}
        downloadEvent={downloadProgress.get(item.video.videoId)}
        downloadBusy={downloadBusyIds.has(item.video.videoId)}
        isPlaying={activeVideoIds.has(item.video.videoId)}
        isLaunching={launchingVideoIds.has(item.video.videoId)}
        {playerReady}
        {downloadsEnabled}
        {sortable}
        {dragging}
        {suppressClick}
        {onReorderStart}
        onRemove={(video) => selected && onRemove(selected.id, video)}
      />
    {/if}
  {/snippet}
</ListSidebar>
