<script lang="ts">
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import { onDestroy, tick } from 'svelte';
  import { Popover } from 'bits-ui';
  import {
    ArrowDown,
    ArrowUp,
    ArrowUpDown,
    Check,
    ChevronDown,
    LoaderCircle,
    Plus,
    Settings2,
    Trash2,
    X,
  } from '@lucide/svelte';
  import type {
    YoutubeDownloadEvent,
    YoutubeVideo,
    YoutubeWatchlist,
    YoutubeWatchlistUpdate,
  } from '../../types';
  import { createLayoutMotion, type LayoutSnapshot } from '../../layout-motion';
  import { sortedYoutubeWatchlistItems } from '../../youtube-watchlists';
  import Button from '../ui/Button.svelte';
  import Switch from '../ui/Switch.svelte';
  import WatchlistItem from './WatchlistItem.svelte';

  let {
    actions,
    watchlists,
    selectedWatchlistId,
    downloadProgress,
    downloadBusyIds,
    activeVideoIds,
    launchingVideoIds,
    mpvReady,
    ytdlpReady,
    ffmpegReady,
    onSelect,
    onCreate,
    onUpdate,
    onDelete,
    onReorder,
    onRemove,
  }: {
    actions: YoutubeVideoActions;
    watchlists: YoutubeWatchlist[];
    selectedWatchlistId: number | null;
    downloadProgress: Map<string, YoutubeDownloadEvent>;
    downloadBusyIds: Set<string>;
    activeVideoIds: Set<string>;
    launchingVideoIds: Set<string>;
    mpvReady: boolean;
    ytdlpReady: boolean;
    ffmpegReady: boolean;
    onSelect: (watchlistId: number) => void;
    onCreate: (name: string) => Promise<void>;
    onUpdate: (
      watchlistId: number,
      patch: Partial<YoutubeWatchlistUpdate>,
    ) => Promise<void>;
    onDelete: (watchlistId: number) => Promise<void>;
    onReorder: (watchlistId: number, videoIds: string[]) => Promise<void>;
    onRemove: (watchlistId: number, video: YoutubeVideo) => void;
  } = $props();

  let listMenuOpen = $state(false);
  let sortMenuOpen = $state(false);
  let settingsOpen = $state(false);
  let confirmingDeleteWatchlistId = $state<number | null>(null);
  let newListName = $state('');
  let draggingVideoId = $state<string | null>(null);
  let dragItems = $state<string[] | null>(null);
  let dragSnapshot: LayoutSnapshot | null = null;
  let suppressDragClickVideoId = $state<string | null>(null);
  let dragCandidate: {
    pointerId: number;
    videoId: string;
    source: HTMLElement;
    startX: number;
    startY: number;
    offsetX: number;
    offsetY: number;
  } | null = null;
  let dragPreview: HTMLElement | null = null;
  let dragPreviewScale = 1;
  let watchlistListElement = $state<HTMLElement | null>(null);
  let previousBodyUserSelect: string | null = null;
  const motion = createLayoutMotion();
  const connectMotion = motion.connect;
  let previousMotionWatchlistId: number | null | undefined = undefined;
  let previousMotionItemIds: string[] = [];
  let membershipMotionGeneration = 0;
  const selected = $derived(
    watchlists.find((watchlist) => watchlist.id === selectedWatchlistId) ??
      watchlists[0],
  );
  let renameValue = $derived(selected?.name ?? '');
  const canReorder = $derived(
    selected?.sortMode === 'manual' &&
      !selected.items.some((item) => item.metadataPending),
  );
  const items = $derived.by(() => {
    if (!selected) return [];
    return canReorder && dragItems
      ? dragItems
          .map((videoId) =>
            selected.items.find((item) => item.video.videoId === videoId),
          )
          .filter((item) => item !== undefined)
      : sortedYoutubeWatchlistItems(selected);
  });

  $effect.pre(() => {
    const watchlistId = selected?.id ?? null;
    const itemIds = items.map((item) => item.video.videoId);
    const previousIds = new Set(previousMotionItemIds);
    const nextIds = new Set(itemIds);
    const sameWatchlist =
      previousMotionWatchlistId !== undefined &&
      watchlistId === previousMotionWatchlistId;
    const membershipChanged =
      !sameWatchlist ||
      itemIds.length !== previousMotionItemIds.length ||
      itemIds.some((videoId) => !previousIds.has(videoId));
    const removedItem =
      sameWatchlist &&
      previousMotionItemIds.some((videoId) => !nextIds.has(videoId));
    const orderChanged =
      sameWatchlist &&
      !membershipChanged &&
      itemIds.some(
        (videoId, index) => videoId !== previousMotionItemIds[index],
      );

    previousMotionWatchlistId = watchlistId;
    previousMotionItemIds = itemIds;
    if (membershipChanged || orderChanged) membershipMotionGeneration += 1;
    if ((!removedItem && !orderChanged) || draggingVideoId) return;

    const snapshot = motion.capture();
    if (!snapshot) return;
    const generation = membershipMotionGeneration;
    void tick().then(() => {
      if (generation === membershipMotionGeneration) motion.play(snapshot);
    });
  });

  async function createList() {
    const name = newListName.trim();
    if (!name) return;
    await onCreate(name);
    newListName = '';
    listMenuOpen = false;
  }

  async function updateSelected(patch: Partial<YoutubeWatchlistUpdate>) {
    if (!selected) return;
    await onUpdate(selected.id, patch);
  }

  function removeDragPreview() {
    dragPreview?.remove();
    dragPreview = null;
    dragPreviewScale = 1;
  }

  function disableTextSelection() {
    window.getSelection()?.removeAllRanges();
    if (previousBodyUserSelect !== null) return;
    previousBodyUserSelect = document.body.style.userSelect;
    document.body.style.userSelect = 'none';
  }

  function restoreTextSelection() {
    if (previousBodyUserSelect === null) return;
    document.body.style.userSelect = previousBodyUserSelect;
    previousBodyUserSelect = null;
  }

  function removePointerDragListeners() {
    window.removeEventListener('pointermove', movePointerDrag);
    window.removeEventListener('pointerup', finishPointerDrag);
    window.removeEventListener('pointercancel', cancelPointerDrag);
  }

  function beginPointerDrag(event: PointerEvent, videoId: string) {
    if (
      !selected ||
      !canReorder ||
      event.button !== 0 ||
      (event.target as HTMLElement).closest(
        'button, input, textarea, select, a',
      )
    ) {
      return;
    }
    const source = event.currentTarget as HTMLElement;
    const rect = source.getBoundingClientRect();
    source.setPointerCapture(event.pointerId);
    dragCandidate = {
      pointerId: event.pointerId,
      videoId,
      source,
      startX: event.clientX,
      startY: event.clientY,
      offsetX: event.clientX - rect.left,
      offsetY: event.clientY - rect.top,
    };
    window.addEventListener('pointermove', movePointerDrag);
    window.addEventListener('pointerup', finishPointerDrag);
    window.addEventListener('pointercancel', cancelPointerDrag);
  }

  function startPointerDrag(event: PointerEvent) {
    if (!dragCandidate || !selected) return;
    disableTextSelection();
    draggingVideoId = dragCandidate.videoId;
    dragItems = selected.items.map((item) => item.video.videoId);

    const rect = dragCandidate.source.getBoundingClientRect();
    const scale =
      Number.parseFloat(
        getComputedStyle(dragCandidate.source).getPropertyValue(
          '--youtube-media-scale',
        ),
      ) || 1;
    dragPreviewScale = scale;
    const preview = dragCandidate.source.cloneNode(true) as HTMLElement;
    preview.removeAttribute('data-layout-key');
    preview.removeAttribute('data-watchlist-video-id');
    preview.setAttribute('aria-hidden', 'true');
    preview.inert = true;
    Object.assign(preview.style, {
      position: 'fixed',
      zIndex: '1000',
      left: `${(event.clientX - dragCandidate.offsetX) / scale}px`,
      top: `${(event.clientY - dragCandidate.offsetY) / scale}px`,
      width: `${rect.width / scale}px`,
      height: `${rect.height / scale}px`,
      zoom: String(scale),
      margin: '0',
      pointerEvents: 'none',
      opacity: '0.96',
      background: 'var(--surface)',
      boxShadow: '0 14px 34px var(--shadow)',
      border: '1px solid var(--line-strong)',
    });
    preview.classList.add('provider-overlay');
    document.body.appendChild(preview);
    dragPreview = preview;
  }

  async function movePointerDrag(event: PointerEvent) {
    if (!dragCandidate || dragCandidate.pointerId !== event.pointerId) return;
    if (!draggingVideoId) {
      const distance = Math.hypot(
        event.clientX - dragCandidate.startX,
        event.clientY - dragCandidate.startY,
      );
      if (distance < 5) return;
      startPointerDrag(event);
    }
    if (!draggingVideoId || !dragItems || !dragPreview) return;
    event.preventDefault();
    dragPreview.style.left = `${(event.clientX - dragCandidate.offsetX) / dragPreviewScale}px`;
    dragPreview.style.top = `${(event.clientY - dragCandidate.offsetY) / dragPreviewScale}px`;

    const targetElement = document
      .elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>('[data-watchlist-video-id]');
    const targetVideoId = targetElement?.dataset.watchlistVideoId;
    const from = dragItems.indexOf(draggingVideoId);
    if (from < 0) return;

    let to: number | null = null;
    if (targetElement && targetVideoId && targetVideoId !== draggingVideoId) {
      const target = dragItems.indexOf(targetVideoId);
      if (target < 0 || from === target) return;
      const rect = targetElement.getBoundingClientRect();
      const after = event.clientY > rect.top + rect.height / 2;
      to = target + (after ? 1 : 0);
    } else if (watchlistListElement) {
      const rows = [
        ...watchlistListElement.querySelectorAll<HTMLElement>(
          '[data-watchlist-video-id]',
        ),
      ];
      if (rows.length === 0) return;

      const firstRect = rows[0].getBoundingClientRect();
      const lastRect = rows[rows.length - 1].getBoundingClientRect();
      if (event.clientY <= firstRect.top) {
        to = 0;
      } else if (event.clientY >= lastRect.bottom) {
        to = dragItems.length;
      } else {
        const nextRow = rows.find((row) => {
          const rect = row.getBoundingClientRect();
          return event.clientY < rect.top + rect.height / 2;
        });
        const nextVideoId = nextRow?.dataset.watchlistVideoId;
        if (nextVideoId) to = dragItems.indexOf(nextVideoId);
      }
    }

    if (to === null || to < 0) return;
    if (from < to) to -= 1;
    if (from === to) return;
    dragSnapshot = motion.capture();
    const next = [...dragItems];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    dragItems = next;
    await tick();
    motion.play(dragSnapshot);
  }

  async function finishPointerDrag(event: PointerEvent) {
    if (!dragCandidate || dragCandidate.pointerId !== event.pointerId) return;
    const list = selected;
    const order = dragItems;
    const draggedVideoId = draggingVideoId;
    if (dragCandidate.source.hasPointerCapture(event.pointerId))
      dragCandidate.source.releasePointerCapture(event.pointerId);
    removePointerDragListeners();
    removeDragPreview();
    restoreTextSelection();
    dragCandidate = null;
    draggingVideoId = null;
    dragItems = null;
    if (draggedVideoId) {
      suppressDragClickVideoId = draggedVideoId;
      window.setTimeout(() => {
        if (suppressDragClickVideoId === draggedVideoId)
          suppressDragClickVideoId = null;
      }, 0);
    }
    if (list && order && draggedVideoId && canReorder)
      await onReorder(list.id, order);
  }

  function cancelPointerDrag(event: PointerEvent) {
    if (!dragCandidate || dragCandidate.pointerId !== event.pointerId) return;
    if (dragCandidate.source.hasPointerCapture(event.pointerId))
      dragCandidate.source.releasePointerCapture(event.pointerId);
    removePointerDragListeners();
    removeDragPreview();
    restoreTextSelection();
    dragCandidate = null;
    draggingVideoId = null;
    dragItems = null;
  }

  onDestroy(() => {
    removePointerDragListeners();
    removeDragPreview();
    restoreTextSelection();
  });
</script>

<aside
  class="flex h-full min-h-0 w-full flex-col border-l border-(--line) bg-(--surface)"
>
  <div
    data-sidebar-resize="y"
    data-sidebar-resize-zoom
    class="flex shrink-0 items-center gap-1.5 border-b border-(--line) p-2"
  >
    <div class="relative min-w-0 flex-1">
      <Popover.Root
        bind:open={listMenuOpen}
        onOpenChange={(open) => {
          if (!open) confirmingDeleteWatchlistId = null;
          else {
            sortMenuOpen = false;
            settingsOpen = false;
          }
        }}
      >
        <Popover.Trigger
          type="button"
          class="flex h-9 w-full items-center gap-2 border border-(--line) px-2.5 text-left hover:border-(--line-strong)"
        >
          <span class="min-w-0 flex-1 truncate text-sm"
            >{selected?.name ?? 'Watch Later'}</span
          >
          <span class="font-mono text-[0.6rem] text-(--muted)"
            >{selected?.items.length ?? 0}</span
          >
          <ChevronDown class="size-3.5 text-(--muted)" />
        </Popover.Trigger>
        <Popover.ContentStatic
          role="dialog"
          aria-label="Watchlists"
          class="panel absolute top-10 right-0 left-0 z-50 border border-(--line-strong) bg-(--surface-strong) p-1.5 shadow-xl"
        >
          {#each watchlists as watchlist (watchlist.id)}
            <div
              class={`flex min-w-0 items-center ${watchlist.id === selected?.id ? 'bg-(--accent-soft) text-(--accent)' : 'hover:bg-(--surface)'}`}
            >
              <button
                type="button"
                class="flex h-9 min-w-0 flex-1 items-center px-2.5 text-left text-sm"
                onclick={() => {
                  onSelect(watchlist.id);
                  confirmingDeleteWatchlistId = null;
                  listMenuOpen = false;
                }}
              >
                <span class="min-w-0 flex-1 truncate">{watchlist.name}</span>
                <span class="ml-2 font-mono text-[0.6rem] text-(--muted)"
                  >{watchlist.items.length}</span
                >
              </button>
              {#if !watchlist.isDefault}
                {#if confirmingDeleteWatchlistId === watchlist.id}
                  <div
                    class="mr-1 grid shrink-0 grid-cols-2 gap-1"
                    role="group"
                    aria-label={`Delete ${watchlist.name} confirmation`}
                  >
                    <button
                      type="button"
                      class="grid size-7 place-items-center border border-[color-mix(in_srgb,var(--success)_55%,var(--line-strong))] bg-[color-mix(in_srgb,var(--success)_12%,transparent)] text-(--success) hover:bg-[color-mix(in_srgb,var(--success)_20%,transparent)]"
                      title={`Confirm deleting ${watchlist.name}`}
                      aria-label={`Confirm deleting ${watchlist.name}`}
                      onclick={() => {
                        confirmingDeleteWatchlistId = null;
                        listMenuOpen = false;
                        void onDelete(watchlist.id);
                      }}
                    >
                      <Check class="size-3.5" />
                    </button>
                    <button
                      type="button"
                      class="grid size-7 place-items-center border border-[color-mix(in_srgb,var(--danger)_55%,var(--line-strong))] bg-[color-mix(in_srgb,var(--danger)_12%,transparent)] text-(--danger) hover:bg-[color-mix(in_srgb,var(--danger)_20%,transparent)]"
                      title={`Cancel deleting ${watchlist.name}`}
                      aria-label={`Cancel deleting ${watchlist.name}`}
                      onclick={() => (confirmingDeleteWatchlistId = null)}
                    >
                      <X class="size-3.5" />
                    </button>
                  </div>
                {:else}
                  <button
                    type="button"
                    class="mr-1 grid size-7 shrink-0 place-items-center text-(--muted) hover:bg-[color-mix(in_srgb,var(--danger)_10%,transparent)] hover:text-(--danger)"
                    title={`Delete ${watchlist.name}`}
                    aria-label={`Delete ${watchlist.name}`}
                    onclick={() => (confirmingDeleteWatchlistId = watchlist.id)}
                  >
                    <Trash2 class="size-3.5" />
                  </button>
                {/if}
              {/if}
            </div>
          {/each}
          <div class="mt-1 flex gap-1 border-t border-(--line) pt-1.5">
            <input
              class="min-w-0 flex-1 border border-(--line) bg-(--background) px-2 text-xs outline-none focus:border-(--line-strong)"
              placeholder="New watchlist"
              bind:value={newListName}
              onkeydown={(event) => event.key === 'Enter' && void createList()}
            />
            <Button
              size="icon"
              variant="ghost"
              disabled={!newListName.trim()}
              onclick={createList}
              title="Create watchlist"><Plus class="size-4" /></Button
            >
          </div>
        </Popover.ContentStatic>
      </Popover.Root>
    </div>
    <div class="relative">
      <Popover.Root
        bind:open={sortMenuOpen}
        onOpenChange={(open) => {
          if (open) {
            listMenuOpen = false;
            confirmingDeleteWatchlistId = null;
            settingsOpen = false;
          }
        }}
      >
        <Popover.Trigger>
          {#snippet child({ props })}
            <Button
              {...props}
              size="icon"
              variant={sortMenuOpen ? 'secondary' : 'ghost'}
              aria-label="Sort watchlist"
              title="Sort watchlist"
              aria-expanded={sortMenuOpen}
              ><ArrowUpDown class="size-4" /></Button
            >
          {/snippet}
        </Popover.Trigger>
        {#if selected}
          <Popover.ContentStatic
            role="dialog"
            aria-label="Sort watchlist"
            class="panel absolute top-10 right-0 z-50 w-64 border border-(--line-strong) bg-(--surface-strong) p-3 shadow-xl"
          >
            <p class="eyebrow mb-2">SORT BY</p>
            <div class="flex items-center gap-1 py-1.5">
              <span class="flex-1 text-sm">Manual order</span>
              <button
                type="button"
                class={`h-8 border px-2.5 text-xs ${selected.sortMode === 'manual' ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted) hover:text-(--foreground)'}`}
                aria-pressed={selected.sortMode === 'manual'}
                onclick={() => void updateSelected({ sortMode: 'manual' })}
                >Manual</button
              >
            </div>
            <div
              class="flex items-center gap-1 border-t border-(--line) py-1.5"
            >
              <span class="flex-1 text-sm">Date</span>
              <button
                type="button"
                class={`grid size-8 place-items-center border ${selected.sortMode === 'date' && selected.sortDirection === 'asc' ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted) hover:text-(--foreground)'}`}
                aria-label="Date ascending"
                aria-pressed={selected.sortMode === 'date' &&
                  selected.sortDirection === 'asc'}
                onclick={() =>
                  void updateSelected({
                    sortMode: 'date',
                    sortDirection: 'asc',
                  })}><ArrowUp class="size-3.5" /></button
              >
              <button
                type="button"
                class={`grid size-8 place-items-center border ${selected.sortMode === 'date' && selected.sortDirection === 'desc' ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted) hover:text-(--foreground)'}`}
                aria-label="Date descending"
                aria-pressed={selected.sortMode === 'date' &&
                  selected.sortDirection === 'desc'}
                onclick={() =>
                  void updateSelected({
                    sortMode: 'date',
                    sortDirection: 'desc',
                  })}><ArrowDown class="size-3.5" /></button
              >
            </div>
          </Popover.ContentStatic>
        {/if}
      </Popover.Root>
    </div>
    <div class="relative">
      <Popover.Root
        bind:open={settingsOpen}
        onOpenChange={(open) => {
          if (open) {
            listMenuOpen = false;
            confirmingDeleteWatchlistId = null;
            sortMenuOpen = false;
          }
        }}
      >
        <Popover.Trigger>
          {#snippet child({ props })}
            <Button
              {...props}
              size="icon"
              variant={settingsOpen ? 'secondary' : 'ghost'}
              title="Watchlist settings"
              aria-label="Watchlist settings"
              aria-expanded={settingsOpen}><Settings2 class="size-4" /></Button
            >
          {/snippet}
        </Popover.Trigger>
        {#if selected}
          <Popover.ContentStatic
            role="dialog"
            aria-label="Watchlist settings"
            class="panel absolute top-10 right-0 z-50 w-72 border border-(--line-strong) bg-(--surface-strong) p-3 text-xs shadow-xl"
          >
            <p class="eyebrow mb-2">WATCHLIST SETTINGS</p>
            {#if !selected.isDefault}
              <label class="mb-2 block">
                <span class="mb-1 block text-(--muted)">Name</span>
                <input
                  class="h-8 w-full border border-(--line) bg-(--background) px-2 outline-none focus:border-(--line-strong)"
                  maxlength="80"
                  bind:value={renameValue}
                  onkeydown={(event) => {
                    if (
                      event.key === 'Enter' &&
                      renameValue.trim() &&
                      renameValue.trim() !== selected.name
                    )
                      void updateSelected({ name: renameValue.trim() });
                  }}
                  onblur={() => {
                    if (
                      renameValue.trim() &&
                      renameValue.trim() !== selected.name
                    )
                      void updateSelected({ name: renameValue.trim() });
                    else renameValue = selected.name;
                  }}
                />
              </label>
            {/if}
            <Switch
              size="sm"
              class="flex border-t border-(--line) py-2"
              checked={selected.autoDownload}
              onCheckedChange={(checked) =>
                void updateSelected({ autoDownload: checked })}
            >
              Automatically download added videos
            </Switch>
            <Switch
              size="sm"
              class="flex border-t border-(--line) py-2"
              checked={selected.autoRemoveWatched}
              onCheckedChange={(checked) =>
                void updateSelected({ autoRemoveWatched: checked })}
            >
              Remove videos when watched
            </Switch>
          </Popover.ContentStatic>
        {/if}
      </Popover.Root>
    </div>
  </div>

  {#if selected}
    <div
      bind:this={watchlistListElement}
      use:connectMotion
      data-sidebar-resize="y-pos"
      data-sidebar-resize-zoom
      role="list"
      class="relative min-h-0 flex-1 [scrollbar-color:var(--line-strong)_transparent] overflow-y-auto"
    >
      {#if items.length === 0}
        <div
          class="grid min-h-36 place-items-center px-6 text-center text-xs leading-5 text-(--muted)"
        >
          Add videos from the ⋯ menu on any YouTube card.
        </div>
      {:else}
        <div
          data-sidebar-resize="y-scale-scroll"
          data-sidebar-resize-zoom
          role="presentation"
        >
          {#each items as item (item.video.videoId)}
            <div
              role="listitem"
              data-layout-key={item.video.videoId}
              data-watchlist-video-id={item.video.videoId}
              onpointerdown={(event) =>
                beginPointerDrag(event, item.video.videoId)}
            >
              {#if item.metadataPending}
                <div
                  class="flex items-stretch border-b border-(--line)"
                  aria-busy="true"
                >
                  <div
                    class="grid aspect-video w-[min(10rem,42%)] shrink-0 place-items-center bg-[linear-gradient(130deg,#121923,#080b10_60%)]"
                  >
                    <LoaderCircle class="size-5 animate-spin text-(--accent)" />
                  </div>
                  <div class="min-w-0 flex-1 px-2.5 py-1">
                    <p class="text-sm leading-5 font-medium">Loading video…</p>
                    <p class="mt-0.5 truncate text-[0.62rem] text-(--muted)">
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
                  {mpvReady}
                  {ytdlpReady}
                  {ffmpegReady}
                  sortable={canReorder}
                  dragging={draggingVideoId === item.video.videoId}
                  suppressClick={suppressDragClickVideoId ===
                    item.video.videoId}
                  onRemove={(video) => onRemove(selected.id, video)}
                />
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</aside>
