<script lang="ts" generics="T extends { id: number; name: string }">
  import { tick } from 'svelte';
  import { flip } from 'svelte/animate';
  import { prefersReducedMotion } from 'svelte/motion';
  import { GripVertical } from '@lucide/svelte';
  import Switch from './Switch.svelte';

  let {
    items = $bindable(),
    enabled,
    label,
    onToggle,
  } = $props<{
    items: T[];
    enabled: number[];
    label: string;
    onToggle: (id: number, enabled: boolean) => void;
  }>();
  const id = $props.id();
  let list: HTMLOListElement;
  let dragControl: HTMLButtonElement;
  let announcement = $state('');
  let drag = $state<{
    id: number;
    pointer: number;
    from: number;
    to: number;
    startY: number;
    startX: number;
    y: number;
    x: number;
    offsetX: number;
    offsetY: number;
    width: number;
    height: number;
    active: boolean;
  } | null>(null);

  // Preview the entire order without committing it until the pointer is released.
  const displayed = $derived.by(() => {
    const next: T[] = [...items];
    if (drag?.active) {
      const [item] = next.splice(drag.from, 1);
      next.splice(drag.to, 0, item);
    }
    return next;
  });

  async function reorder(from: number, to: number) {
    if (from === to) return;
    const next = [...items];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    items = next;
    announcement = `${item.name}, priority ${to + 1} of ${items.length}`;
    await tick();
    list
      .querySelector<HTMLButtonElement>(`[data-grip="${item.id}"]`)
      ?.focus({ preventScroll: true });
  }
  function start(event: PointerEvent, index: number) {
    if (event.button !== 0 || !event.isPrimary) return;
    const handle = event.currentTarget as HTMLButtonElement;
    const rect = handle.closest('li')!.getBoundingClientRect();
    handle.focus({ preventScroll: true });
    // Capture on a stable control: moving the keyed row would release its capture.
    dragControl.setPointerCapture(event.pointerId);
    drag = {
      id: items[index].id,
      pointer: event.pointerId,
      from: index,
      to: index,
      startY: event.clientY,
      startX: event.clientX,
      y: event.clientY,
      x: event.clientX,
      offsetX: event.clientX - rect.x,
      offsetY: event.clientY - rect.y,
      width: rect.width,
      height: rect.height,
      active: false,
    };
  }
  function locate() {
    if (!drag) return;
    const rows = [
      ...list.querySelectorAll<HTMLLIElement>(':scope > li'),
    ].filter((row) => Number(row.dataset.item) !== drag!.id);
    const top = list.getBoundingClientRect().top - list.scrollTop;
    // Hit-test layout slots, not the animated positions moving between them.
    const index = rows.findIndex(
      (row) => drag!.y < top + row.offsetTop + row.offsetHeight / 2,
    );
    drag.to = index < 0 ? rows.length : index;
  }
  function move(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.pointer) return;
    drag.y = event.clientY;
    drag.x = event.clientX;
    if (
      !drag.active &&
      Math.hypot(drag.x - drag.startX, drag.y - drag.startY) >= 4
    ) {
      drag.active = true;
      dragControl.focus({ preventScroll: true });
    }
    if (drag.active) {
      event.preventDefault();
      locate();
    }
  }
  async function finish(commit: boolean) {
    if (!drag) return;
    const current = drag;
    drag = null;
    if (dragControl.hasPointerCapture(current.pointer))
      dragControl.releasePointerCapture(current.pointer);
    if (commit && current.active) await reorder(current.from, current.to);
    await tick();
    list
      .querySelector<HTMLButtonElement>(`[data-grip="${current.id}"]`)
      ?.focus({ preventScroll: true });
  }
  function key(event: KeyboardEvent, index: number) {
    if (event.key === 'Escape' && drag) {
      event.preventDefault();
      event.stopPropagation();
      void finish(false);
      return;
    }
    const to =
      event.key === 'ArrowUp'
        ? index - 1
        : event.key === 'ArrowDown'
          ? index + 1
          : event.key === 'Home'
            ? 0
            : event.key === 'End'
              ? items.length - 1
              : null;
    if (to !== null && !drag) {
      event.preventDefault();
      void reorder(index, Math.max(0, Math.min(items.length - 1, to)));
    }
  }
  // Keep dragging usable when a long list extends beyond the visible rows.
  $effect(() => {
    if (!drag?.active) return;
    let frame = 0;
    function scroll() {
      if (!drag) return;
      const rect = list.getBoundingClientRect();
      const speed =
        drag.y < rect.top + 32 ? -8 : drag.y > rect.bottom - 32 ? 8 : 0;
      if (speed) {
        list.scrollTop += speed;
        locate();
      }
      frame = requestAnimationFrame(scroll);
    }
    frame = requestAnimationFrame(scroll);
    return () => cancelAnimationFrame(frame);
  });
</script>

{#snippet rowContent(item: T, index: number)}
  <button
    type="button"
    data-grip={item.id}
    aria-label={`Reorder ${item.name}`}
    aria-describedby={id}
    class="grid w-9 shrink-0 touch-none self-stretch cursor-grab place-items-center text-muted outline-offset-1 hover:text-foreground focus-visible:outline-2 focus-visible:outline-accent active:cursor-grabbing"
    onpointerdown={(event) => start(event, index)}
    onkeydown={(event) => key(event, index)}><GripVertical size={16} /></button
  >
  <Switch
    class="min-w-0 flex-1 flex-row-reverse justify-between gap-3 py-2 pr-3 text-xs"
    size="sm"
    checked={enabled.includes(item.id)}
    onCheckedChange={(checked) => onToggle(item.id, checked)}
    >{item.name}</Switch
  >
{/snippet}

<p {id} class="sr-only">
  Drag a handle to reorder, or focus it and use the arrow keys. Highest priority
  is first.
</p>
<button
  bind:this={dragControl}
  type="button"
  tabindex="-1"
  class="sr-only"
  aria-label="Reordering quality; press Escape to cancel"
  onpointermove={move}
  onpointerup={() => void finish(true)}
  onpointercancel={() => void finish(false)}
  onlostpointercapture={() => void finish(false)}
  onkeydown={(event) => key(event, drag?.to ?? 0)}
></button>
<ol
  bind:this={list}
  aria-label={label}
  class="relative grid max-h-[min(55dvh,32rem)] gap-1 overflow-y-auto overscroll-contain p-1"
>
  {#each displayed as item, index (item.id)}
    <li
      data-item={item.id}
      data-drop-placeholder={drag?.active && drag.id === item.id
        ? ''
        : undefined}
      inert={drag?.active && drag.id === item.id}
      animate:flip={{
        duration:
          prefersReducedMotion.current || (drag?.active && drag.id === item.id)
            ? 0
            : 180,
      }}
      class={[
        'relative flex min-h-10 items-center border',
        drag?.active && drag.id === item.id
          ? 'border-dashed border-accent bg-accent/10 [&>*]:opacity-30'
          : 'border-line bg-surface-soft',
      ]}
    >
      {@render rowContent(item, index)}
    </li>
  {/each}
</ol>
{#if drag?.active}
  <div
    data-drag-preview
    aria-hidden="true"
    inert
    class="pointer-events-none fixed z-20 flex items-center border border-accent bg-surface-strong shadow-xl"
    style:left={`${drag.x - drag.offsetX}px`}
    style:top={`${drag.y - drag.offsetY}px`}
    style:width={`${drag.width}px`}
    style:height={`${drag.height}px`}
  >
    {@render rowContent(items[drag.from], drag.from)}
  </div>
{/if}
<span class="sr-only" role="status">{announcement}</span>
