<script lang="ts">
  import type { Snippet } from 'svelte';
  import { GripVertical } from '@lucide/svelte';
  import { longPress } from '../long-press';
  import { touchDevice } from '../responsive';

  let {
    title,
    canPlay,
    isPlaying = false,
    sortable = false,
    dragging = false,
    suppressClick = false,
    closeActionsLabel,
    class: className = '',
    textClass = 'py-1',
    titleClass = 'line-clamp-2',
    onPlay,
    onReorderStart,
    thumbnail,
    titleEnd,
    details,
    actions,
  }: {
    title: string;
    canPlay: boolean;
    isPlaying?: boolean;
    sortable?: boolean;
    dragging?: boolean;
    suppressClick?: boolean;
    closeActionsLabel: string;
    class?: string;
    textClass?: string;
    titleClass?: string;
    onPlay: () => void;
    onReorderStart?: (event: PointerEvent) => void;
    thumbnail: Snippet;
    /** What shares the title's line, after it. */
    titleEnd?: Snippet;
    /** What goes under the title. */
    details: Snippet;
    /** Hover buttons, or the rows of a touch device's long-press sheet. */
    actions: Snippet<[{ actionClass: string; close: () => void }]>;
  } = $props();

  let titleTruncated = $state(false);
  let actionsOpen = $state(false);
  let actionMenu = $state<HTMLDivElement | null>(null);
  const actionClass = $derived(
    touchDevice.current
      ? 'flex h-11 w-full items-center gap-3 px-4 text-left text-sm text-muted hover:bg-surface-strong hover:text-foreground'
      : 'grid size-7 place-items-center text-muted hover:bg-surface-strong hover:text-foreground',
  );
  $effect(() => {
    if (!actionMenu) return;
    if (touchDevice.current && actionsOpen) actionMenu.showPopover();
    else if (actionMenu.matches(':popover-open')) actionMenu.hidePopover();
  });

  function trackTitleOverflow(node: HTMLElement) {
    const update = () => {
      titleTruncated =
        node.scrollHeight > node.clientHeight + 1 ||
        node.scrollWidth > node.clientWidth + 1;
    };
    const resizeObserver = new ResizeObserver(update);
    const mutationObserver = new MutationObserver(update);
    resizeObserver.observe(node);
    mutationObserver.observe(node, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    requestAnimationFrame(update);

    return {
      destroy() {
        resizeObserver.disconnect();
        mutationObserver.disconnect();
      },
    };
  }

  function clickCard(event: MouseEvent) {
    if (
      suppressClick ||
      !canPlay ||
      (event.target as HTMLElement).closest('button')
    )
      return;
    onPlay();
  }

  function keyCard(event: KeyboardEvent) {
    if (
      event.target !== event.currentTarget ||
      !canPlay ||
      !['Enter', ' '].includes(event.key)
    )
      return;
    event.preventDefault();
    onPlay();
  }
</script>

<div
  use:longPress={() => (actionsOpen = true)}
  data-playing={isPlaying}
  class={`group/list-item relative flex cursor-default items-stretch border-b border-line transition-[opacity,background-color] duration-(--animation-duration) hover:z-20 touch:select-none touch:[-webkit-touch-callout:none] ${sortable ? 'select-none' : ''} ${sortable && dragging ? 'bg-accent-soft opacity-35' : isPlaying ? 'bg-accent-soft' : sortable || canPlay ? 'hover:bg-surface-soft' : ''} ${className}`}
>
  <div
    role="button"
    tabindex="0"
    aria-disabled={!canPlay}
    aria-label={isPlaying ? `Playing: ${title}` : title}
    class="flex min-w-0 flex-1"
    onclick={clickCard}
    onkeydown={keyCard}
  >
    {#if isPlaying}
      <span
        aria-hidden="true"
        class="pointer-events-none absolute inset-y-0 right-0 z-10 w-1 bg-accent"
      ></span>
    {/if}
    {@render thumbnail()}

    <div class={`relative flex min-w-0 flex-1 flex-col px-2.5 ${textClass}`}>
      <div class="flex min-w-0 items-baseline gap-2">
        <div class="group/title relative min-w-0 flex-1">
          <p
            use:trackTitleOverflow
            class={`text-sm leading-5 font-medium ${titleClass}`}
          >
            {title}
          </p>
          {#if titleTruncated && !dragging}
            <div
              class="pointer-events-none absolute -top-1.5 -right-1.5 -left-1.5 z-30 translate-y-0.5 border border-line-strong bg-surface-strong px-2 py-1.5 text-sm leading-5 font-medium opacity-0 shadow-[0_12px_30px_var(--shadow)] transition-[opacity,translate] delay-0 duration-(--animation-duration) ease-out group-hover/title:translate-y-0 group-hover/title:opacity-100 group-hover/title:delay-750 motion-reduce:transition-none"
              aria-hidden="true"
            >
              {title}
            </div>
          {/if}
        </div>
        {@render titleEnd?.()}
      </div>
      {@render details()}
    </div>
  </div>
  {#if touchDevice.current && sortable}
    <button
      type="button"
      data-long-press-ignore
      aria-label={`Reorder ${title}`}
      aria-pressed={dragging}
      class="flex w-11 shrink-0 items-center justify-center text-muted active:text-accent"
      onpointerdown={onReorderStart}
      oncontextmenu={(event) => event.preventDefault()}
      ><GripVertical class="size-5" /></button
    >
  {/if}
  {#if touchDevice.current && actionsOpen}
    <button
      type="button"
      data-long-press-ignore
      class="fixed inset-0 z-40 cursor-default bg-black/20"
      aria-label={closeActionsLabel}
      onclick={() => (actionsOpen = false)}
    ></button>
  {/if}
  <div
    bind:this={actionMenu}
    data-long-press-ignore
    popover={touchDevice.current ? 'manual' : undefined}
    role={touchDevice.current ? 'dialog' : undefined}
    aria-label={touchDevice.current ? `Actions for ${title}` : undefined}
    class={touchDevice.current
      ? 'fixed inset-x-0 top-auto bottom-0 z-50 m-0 max-h-[80dvh] w-full max-w-none overflow-y-auto border border-line-strong bg-surface-strong p-2 shadow-2xl'
      : 'invisible absolute right-1 bottom-1 flex gap-0.5 group-hover/list-item:visible group-focus-visible/list-item:visible group-has-focus-visible/list-item:visible has-data-[state=open]:visible'}
    onkeydown={(event) => {
      if (touchDevice.current && event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        actionsOpen = false;
      }
    }}
  >
    {@render actions({ actionClass, close: () => (actionsOpen = false) })}
  </div>
</div>
