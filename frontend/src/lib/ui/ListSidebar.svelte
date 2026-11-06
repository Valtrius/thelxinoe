<script module lang="ts">
  export type ListSidebarRow = {
    sortable: boolean;
    dragging: boolean;
    suppressClick: boolean;
    onReorderStart: (event: PointerEvent) => void;
  };
</script>

<script
  lang="ts"
  generics="List extends { id: string | number; name: string; count: number }, Item"
>
  import { onDestroy, tick, type Snippet } from 'svelte';
  import { Popover } from 'bits-ui';
  import {
    ArrowUpDown,
    Check,
    ChevronDown,
    Plus,
    Settings2,
    Trash2,
    X,
  } from '@lucide/svelte';
  import { createLayoutMotion, type LayoutSnapshot } from '../layout-motion';
  import { touchDevice } from '../responsive';
  import { visibleRange } from '../visible-range';
  import Button from './Button.svelte';
  import MenuBackdrop, { ignoreBackdrop } from './MenuBackdrop.svelte';
  import { toolbarMenuClass } from './styles';

  let {
    noun,
    lists,
    selected,
    items,
    fallbackName,
    emptyText,
    itemKey,
    canReorder,
    rowHeight,
    canDelete,
    listCaption,
    row,
    sortMenu,
    settingsMenu,
    onSelect,
    onCreate,
    onDelete,
    onReorder,
    onClose,
  }: {
    /** What a list is called in labels: "watchlist", "playlist". */
    noun: string;
    lists: List[];
    selected: List | undefined;
    /** The selected list's items, in the order they show. */
    items: Item[];
    fallbackName: string;
    emptyText: string;
    itemKey: (item: Item) => string;
    canReorder: boolean;
    /** Rows of this height render only near the viewport, for lists of any length. */
    rowHeight?: number;
    canDelete: (list: List) => boolean;
    listCaption?: Snippet<[List]>;
    row: Snippet<[Item, ListSidebarRow]>;
    /** Left out when the viewer can't change the list's sort. */
    sortMenu?: Snippet<[List]>;
    /** Left out when the viewer can't change the list's settings. */
    settingsMenu?: Snippet<[List]>;
    onSelect: (id: List['id']) => void;
    /** Resolves to why the list wasn't created, if it wasn't. */
    onCreate: (name: string) => Promise<string | void>;
    onDelete: (id: List['id']) => Promise<void>;
    /** `keys` is the new order, `moved` the item dragged into place. */
    onReorder: (id: List['id'], keys: string[], moved: string) => Promise<void>;
    onClose?: () => void;
  } = $props();

  const Noun = $derived(noun[0].toUpperCase() + noun.slice(1));

  let listMenuOpen = $state(false);
  let sortMenuOpen = $state(false);
  let settingsOpen = $state(false);
  let confirmingDeleteId = $state<List['id'] | null>(null);
  let newListName = $state('');
  let createError = $state('');
  let draggingKey = $state<string | null>(null);
  let dragOrder = $state.raw<string[] | null>(null);
  let dragSnapshot: LayoutSnapshot | null = null;
  let suppressClickKey = $state<string | null>(null);
  let dragHoldTimer: ReturnType<typeof setTimeout> | undefined;
  let dragCandidate: {
    pointerId: number;
    key: string;
    source: HTMLElement;
    startX: number;
    startY: number;
    offsetX: number;
    offsetY: number;
    heldHandle: boolean;
  } | null = null;
  let dragPreview: HTMLElement | null = null;
  let dragPreviewScale = 1;
  let dragPointer = { x: 0, y: 0 };
  let autoScrollFrame: number | null = null;
  let listElement = $state<HTMLElement | null>(null);
  let scrollTop = $state(0);
  let viewportHeight = $state(0);
  let previousBodyUserSelect: string | null = null;
  const motion = createLayoutMotion();
  const connectMotion = motion.connect;
  let previousMotionListId: List['id'] | null | undefined = undefined;
  let previousMotionKeys: string[] = [];
  let membershipMotionGeneration = 0;

  const shown = $derived.by(() => {
    if (!canReorder || !dragOrder) return items;
    const byKey = new Map(items.map((item) => [itemKey(item), item]));
    return dragOrder
      .map((key) => byKey.get(key))
      .filter((item) => item !== undefined);
  });
  const range = $derived(
    rowHeight === undefined
      ? { start: 0, end: shown.length }
      : visibleRange({
          count: shown.length,
          rowHeight,
          scrollTop,
          viewportHeight,
        }),
  );
  const rendered = $derived(shown.slice(range.start, range.end));

  $effect.pre(() => {
    const listId = selected?.id ?? null;
    const keys = shown.map(itemKey);
    const previousKeys = new Set(previousMotionKeys);
    const nextKeys = new Set(keys);
    const sameList =
      previousMotionListId !== undefined && listId === previousMotionListId;
    const membershipChanged =
      !sameList ||
      keys.length !== previousMotionKeys.length ||
      keys.some((key) => !previousKeys.has(key));
    const removedItem =
      sameList && previousMotionKeys.some((key) => !nextKeys.has(key));
    const orderChanged =
      sameList &&
      !membershipChanged &&
      keys.some((key, index) => key !== previousMotionKeys[index]);

    previousMotionListId = listId;
    previousMotionKeys = keys;
    if (membershipChanged || orderChanged) membershipMotionGeneration += 1;
    if ((!removedItem && !orderChanged) || draggingKey) return;

    const snapshot = motion.capture();
    if (!snapshot) return;
    const generation = membershipMotionGeneration;
    void tick().then(() => {
      if (generation === membershipMotionGeneration) motion.play(snapshot);
    });
  });

  // Another list starts from its top.
  let scrolledListId: List['id'] | undefined;
  $effect(() => {
    const listId = selected?.id;
    if (listId === scrolledListId) return;
    scrolledListId = listId;
    if (listElement) listElement.scrollTop = 0;
    scrollTop = 0;
  });

  async function createList() {
    const name = newListName.trim();
    if (!name) return;
    const problem = await onCreate(name);
    if (problem) {
      createError = problem;
      return;
    }
    createError = '';
    newListName = '';
    listMenuOpen = false;
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
    clearTimeout(dragHoldTimer);
    dragHoldTimer = undefined;
    if (autoScrollFrame !== null) cancelAnimationFrame(autoScrollFrame);
    autoScrollFrame = null;
    window.removeEventListener('pointermove', movePointerDrag);
    window.removeEventListener('pointerup', finishPointerDrag);
    window.removeEventListener('pointercancel', cancelPointerDrag);
    window.removeEventListener('touchmove', preventDragScroll);
  }

  function preventDragScroll(event: TouchEvent) {
    if (draggingKey) event.preventDefault();
  }

  function beginPointerDrag(
    event: PointerEvent,
    key: string,
    heldHandle = false,
  ) {
    if (
      !selected ||
      !canReorder ||
      (touchDevice.current && !heldHandle) ||
      event.button !== 0 ||
      (!heldHandle &&
        (event.target as HTMLElement).closest(
          'button, input, textarea, select, a',
        ))
    ) {
      return;
    }
    if (dragCandidate) return;
    const source = heldHandle
      ? (event.currentTarget as HTMLElement).closest<HTMLElement>(
          '[data-list-item-key]',
        )!
      : (event.currentTarget as HTMLElement);
    const rect = source.getBoundingClientRect();
    dragCandidate = {
      pointerId: event.pointerId,
      key,
      source,
      startX: event.clientX,
      startY: event.clientY,
      offsetX: event.clientX - rect.left,
      offsetY: event.clientY - rect.top,
      heldHandle,
    };
    window.addEventListener('pointermove', movePointerDrag);
    window.addEventListener('pointerup', finishPointerDrag);
    window.addEventListener('pointercancel', cancelPointerDrag);
    if (heldHandle) {
      dragHoldTimer = setTimeout(() => {
        dragHoldTimer = undefined;
        startPointerDrag(event);
      }, 350);
      window.addEventListener('touchmove', preventDragScroll, {
        passive: false,
      });
    }
  }

  function startPointerDrag(event: PointerEvent) {
    if (!dragCandidate || !selected) return;
    dragCandidate.source.setPointerCapture(event.pointerId);
    disableTextSelection();
    draggingKey = dragCandidate.key;
    dragOrder = items.map(itemKey);
    dragPointer = { x: event.clientX, y: event.clientY };
    autoScrollFrame = requestAnimationFrame(autoScrollDrag);

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
    preview.removeAttribute('data-list-item-key');
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
    preview.classList.add('provider-overlay', 'text-[1rem]');
    document.body.appendChild(preview);
    dragPreview = preview;
  }

  async function movePointerDrag(event: PointerEvent) {
    if (!dragCandidate || dragCandidate.pointerId !== event.pointerId) return;
    if (!draggingKey) {
      const distance = Math.hypot(
        event.clientX - dragCandidate.startX,
        event.clientY - dragCandidate.startY,
      );
      if (dragCandidate.heldHandle) {
        if (distance > 10) cancelPointerDrag(event);
        return;
      }
      if (distance < 5) return;
      startPointerDrag(event);
    }
    if (!draggingKey || !dragOrder || !dragPreview) return;
    event.preventDefault();
    dragPointer = { x: event.clientX, y: event.clientY };
    dragPreview.style.left = `${(event.clientX - dragCandidate.offsetX) / dragPreviewScale}px`;
    dragPreview.style.top = `${(event.clientY - dragCandidate.offsetY) / dragPreviewScale}px`;
    await dragOver();
  }

  /** Moves the dragged item to where the pointer is over the list. */
  async function dragOver() {
    if (!draggingKey || !dragOrder) return;
    const { x, y } = dragPointer;
    const targetElement = document
      .elementFromPoint(x, y)
      ?.closest<HTMLElement>('[data-list-item-key]');
    const targetKey = targetElement?.dataset.listItemKey;
    const from = dragOrder.indexOf(draggingKey);
    if (from < 0) return;

    let to: number | null = null;
    if (targetElement && targetKey && targetKey !== draggingKey) {
      const target = dragOrder.indexOf(targetKey);
      if (target < 0 || from === target) return;
      const rect = targetElement.getBoundingClientRect();
      const after = y > rect.top + rect.height / 2;
      to = target + (after ? 1 : 0);
    } else if (listElement) {
      // Only rows near the viewport exist, but the pointer stays among them.
      const rows = [
        ...listElement.querySelectorAll<HTMLElement>('[data-list-item-key]'),
      ];
      if (rows.length === 0) return;

      const firstRect = rows[0].getBoundingClientRect();
      const lastRect = rows[rows.length - 1].getBoundingClientRect();
      if (y <= firstRect.top) {
        to = dragOrder.indexOf(rows[0].dataset.listItemKey!);
      } else if (y >= lastRect.bottom) {
        to = dragOrder.indexOf(rows[rows.length - 1].dataset.listItemKey!) + 1;
      } else {
        const nextRow = rows.find((row) => {
          const rect = row.getBoundingClientRect();
          return y < rect.top + rect.height / 2;
        });
        const nextKey = nextRow?.dataset.listItemKey;
        if (nextKey) to = dragOrder.indexOf(nextKey);
      }
    }

    if (to === null || to < 0) return;
    if (from < to) to -= 1;
    if (from === to) return;
    dragSnapshot = motion.capture();
    const next = [...dragOrder];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    dragOrder = next;
    await tick();
    motion.play(dragSnapshot);
  }

  /** Holding near the list's top or bottom scrolls it, to reach rows far away. */
  function autoScrollDrag() {
    autoScrollFrame = requestAnimationFrame(autoScrollDrag);
    if (!listElement || !draggingKey) return;
    const rect = listElement.getBoundingClientRect();
    const zone = Math.min(56, rect.height / 4);
    const { y } = dragPointer;
    const depth =
      y < rect.top + zone
        ? y - (rect.top + zone)
        : y > rect.bottom - zone
          ? y - (rect.bottom - zone)
          : 0;
    if (depth === 0) return;
    const previous = listElement.scrollTop;
    listElement.scrollTop =
      previous + Math.max(-1, Math.min(1, depth / zone)) * 14;
    if (listElement.scrollTop !== previous) void dragOver();
  }

  async function finishPointerDrag(event: PointerEvent) {
    if (!dragCandidate || dragCandidate.pointerId !== event.pointerId) return;
    const list = selected;
    const order = dragOrder;
    const draggedKey = draggingKey;
    if (dragCandidate.source.hasPointerCapture(event.pointerId))
      dragCandidate.source.releasePointerCapture(event.pointerId);
    removePointerDragListeners();
    removeDragPreview();
    restoreTextSelection();
    dragCandidate = null;
    draggingKey = null;
    dragOrder = null;
    if (draggedKey) {
      suppressClickKey = draggedKey;
      window.setTimeout(() => {
        if (suppressClickKey === draggedKey) suppressClickKey = null;
      }, 0);
    }
    if (list && order && draggedKey && canReorder)
      await onReorder(list.id, order, draggedKey);
  }

  function cancelPointerDrag(event: PointerEvent) {
    if (!dragCandidate || dragCandidate.pointerId !== event.pointerId) return;
    if (dragCandidate.source.hasPointerCapture(event.pointerId))
      dragCandidate.source.releasePointerCapture(event.pointerId);
    removePointerDragListeners();
    removeDragPreview();
    restoreTextSelection();
    dragCandidate = null;
    draggingKey = null;
    dragOrder = null;
  }

  onDestroy(() => {
    removePointerDragListeners();
    removeDragPreview();
    restoreTextSelection();
  });
</script>

<aside
  class="flex h-full min-h-0 w-full flex-col border-l border-line bg-surface compact:border-l-0"
>
  <div
    data-sidebar-resize="y"
    data-sidebar-resize-zoom
    class="flex shrink-0 items-center gap-1.5 border-b border-line p-2 compact:relative"
  >
    {#if onClose}
      <Button
        size="icon"
        variant="ghost"
        aria-label={`Close ${noun}s`}
        title={`Close ${noun}s`}
        onclick={onClose}><X class="size-4" /></Button
      >
    {/if}
    <div class="relative min-w-0 flex-1">
      <Popover.Root
        bind:open={listMenuOpen}
        onOpenChange={(open) => {
          if (!open) confirmingDeleteId = null;
          else {
            sortMenuOpen = false;
            settingsOpen = false;
          }
        }}
      >
        <Popover.Trigger
          type="button"
          class="flex h-9 w-full items-center gap-2 border border-line px-2.5 text-left hover:border-line-strong"
        >
          <span class="min-w-0 flex-1 truncate text-sm"
            >{selected?.name ?? fallbackName}</span
          >
          <span class="font-mono text-[0.6rem] text-muted"
            >{selected?.count ?? 0}</span
          >
          <ChevronDown class="size-3.5 text-muted" />
        </Popover.Trigger>
        {#if listMenuOpen}<MenuBackdrop
            close={() => {
              listMenuOpen = false;
              confirmingDeleteId = null;
            }}
          />{/if}
        <Popover.ContentStatic
          role="dialog"
          aria-label={`${Noun}s`}
          onInteractOutside={ignoreBackdrop}
          class="absolute top-10 right-0 left-0 z-50 max-h-[70vh] overflow-y-auto border border-line-strong bg-surface-strong p-1.5 shadow-xl"
        >
          {#each lists as list (list.id)}
            <div
              class={`flex min-w-0 items-center ${list.id === selected?.id ? 'bg-accent-soft text-accent' : 'hover:bg-surface'}`}
            >
              <button
                type="button"
                class="flex h-9 min-w-0 flex-1 items-center px-2.5 text-left text-sm"
                onclick={() => {
                  onSelect(list.id);
                  confirmingDeleteId = null;
                  listMenuOpen = false;
                }}
              >
                <span class="min-w-0 flex-1 truncate">{list.name}</span>
                {#if listCaption}{@render listCaption(list)}{/if}
                <span class="ml-2 font-mono text-[0.6rem] text-muted"
                  >{list.count}</span
                >
              </button>
              {#if canDelete(list)}
                {#if confirmingDeleteId === list.id}
                  <div
                    class="mr-1 grid shrink-0 grid-cols-2 gap-1"
                    role="group"
                    aria-label={`Delete ${list.name} confirmation`}
                  >
                    <button
                      type="button"
                      class="grid size-7 place-items-center border border-[color-mix(in_srgb,var(--success)_55%,var(--line-strong))] bg-success/12 text-success hover:bg-success/20"
                      title={`Confirm deleting ${list.name}`}
                      aria-label={`Confirm deleting ${list.name}`}
                      onclick={() => {
                        confirmingDeleteId = null;
                        listMenuOpen = false;
                        void onDelete(list.id);
                      }}
                    >
                      <Check class="size-3.5" />
                    </button>
                    <button
                      type="button"
                      class="grid size-7 place-items-center border border-[color-mix(in_srgb,var(--danger)_55%,var(--line-strong))] bg-danger/12 text-danger hover:bg-danger/20"
                      title={`Cancel deleting ${list.name}`}
                      aria-label={`Cancel deleting ${list.name}`}
                      onclick={() => (confirmingDeleteId = null)}
                    >
                      <X class="size-3.5" />
                    </button>
                  </div>
                {:else}
                  <button
                    type="button"
                    class="mr-1 grid size-7 shrink-0 place-items-center text-muted hover:bg-danger/10 hover:text-danger"
                    title={`Delete ${list.name}`}
                    aria-label={`Delete ${list.name}`}
                    onclick={() => (confirmingDeleteId = list.id)}
                  >
                    <Trash2 class="size-3.5" />
                  </button>
                {/if}
              {/if}
            </div>
          {/each}
          <div
            class={`flex gap-1 ${lists.length ? 'mt-1 border-t border-line pt-1.5' : ''}`}
          >
            <input
              class="min-w-0 flex-1 border border-line bg-background px-2 text-xs outline-none focus:border-line-strong"
              placeholder={`New ${noun}`}
              bind:value={newListName}
              oninput={() => (createError = '')}
              onkeydown={(event) => event.key === 'Enter' && void createList()}
            />
            <Button
              size="icon"
              variant="ghost"
              disabled={!newListName.trim()}
              onclick={createList}
              title={`Create ${noun}`}><Plus class="size-4" /></Button
            >
          </div>
          {#if createError}<p
              class="mt-1 mb-0 px-1 text-xs text-danger"
              role="alert"
            >
              {createError}
            </p>{/if}
        </Popover.ContentStatic>
      </Popover.Root>
    </div>
    {#if sortMenu}
      <div class="relative compact:static">
        <Popover.Root
          bind:open={sortMenuOpen}
          onOpenChange={(open) => {
            if (open) {
              listMenuOpen = false;
              confirmingDeleteId = null;
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
                aria-label={`Sort ${noun}`}
                title={`Sort ${noun}`}
                aria-expanded={sortMenuOpen}
                ><ArrowUpDown class="size-4" /></Button
              >
            {/snippet}
          </Popover.Trigger>
          {#if selected}
            {#if sortMenuOpen}<MenuBackdrop
                close={() => (sortMenuOpen = false)}
              />{/if}
            <Popover.ContentStatic
              role="dialog"
              aria-label={`Sort ${noun}`}
              onInteractOutside={ignoreBackdrop}
              class={`absolute top-10 right-0 z-50 w-64 border border-line-strong bg-surface-strong p-3 shadow-xl compact:top-12 ${toolbarMenuClass}`}
            >
              {@render sortMenu(selected)}
            </Popover.ContentStatic>
          {/if}
        </Popover.Root>
      </div>
    {/if}
    {#if settingsMenu}
      <div class="relative compact:static">
        <Popover.Root
          bind:open={settingsOpen}
          onOpenChange={(open) => {
            if (open) {
              listMenuOpen = false;
              confirmingDeleteId = null;
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
                title={`${Noun} settings`}
                aria-label={`${Noun} settings`}
                aria-expanded={settingsOpen}
                ><Settings2 class="size-4" /></Button
              >
            {/snippet}
          </Popover.Trigger>
          {#if selected}
            {#if settingsOpen}<MenuBackdrop
                close={() => (settingsOpen = false)}
              />{/if}
            <Popover.ContentStatic
              role="dialog"
              aria-label={`${Noun} settings`}
              onInteractOutside={ignoreBackdrop}
              class={`absolute top-10 right-0 z-50 w-72 border border-line-strong bg-surface-strong p-3 text-xs shadow-xl compact:top-12 ${toolbarMenuClass}`}
            >
              {@render settingsMenu(selected)}
            </Popover.ContentStatic>
          {/if}
        </Popover.Root>
      </div>
    {/if}
  </div>

  {#if selected}
    <div
      bind:this={listElement}
      bind:clientHeight={viewportHeight}
      use:connectMotion
      data-sidebar-resize="y-pos"
      data-sidebar-resize-zoom
      role="list"
      class="relative min-h-0 flex-1 [scrollbar-color:var(--line-strong)_transparent] overflow-y-auto"
      onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
    >
      {#if shown.length === 0}
        <div
          class="grid min-h-36 place-items-center px-6 text-center text-xs leading-5 text-muted"
        >
          {emptyText}
        </div>
      {:else}
        <div
          data-sidebar-resize="y-scale-scroll"
          data-sidebar-resize-zoom
          role="presentation"
          style:height={rowHeight === undefined
            ? undefined
            : `${shown.length * rowHeight}px`}
          style:padding-top={rowHeight === undefined
            ? undefined
            : `${range.start * rowHeight}px`}
        >
          {#each rendered as item (itemKey(item))}
            {@const key = itemKey(item)}
            <div
              role="listitem"
              data-layout-key={key}
              data-list-item-key={key}
              onpointerdown={(event) => beginPointerDrag(event, key)}
            >
              {@render row(item, {
                sortable: canReorder,
                dragging: draggingKey === key,
                suppressClick: suppressClickKey === key,
                onReorderStart: (event) => beginPointerDrag(event, key, true),
              })}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</aside>
