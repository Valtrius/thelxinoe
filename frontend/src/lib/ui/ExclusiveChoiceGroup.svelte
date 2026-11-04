<script lang="ts" generics="Value extends string">
  import { onMount, untrack, type Component } from 'svelte';
  import type { IconProps } from '@lucide/svelte';
  import { playAnimations } from '../layout-animation';

  type Choice = {
    value: Value;
    label: string;
    icon?: Component<Omit<IconProps, 'icon' | 'iconNode'>>;
    count?: number;
  };

  let {
    choices,
    value,
    ariaLabel,
    compact = false,
    tall = false,
    disabled = false,
    resizeWithSidebar = false,
    onChange,
  }: {
    choices: readonly Choice[];
    value: Value;
    ariaLabel: string;
    compact?: boolean;
    /** Matches the height of a toolbar's buttons and fields. */
    tall?: boolean;
    disabled?: boolean;
    resizeWithSidebar?: boolean;
    onChange: (value: Value) => void;
  } = $props();
  let group = $state<HTMLDivElement>();
  let selection = $state({ left: 0, width: 0 });
  let highlight: HTMLSpanElement;
  let requested: Value | undefined;
  let animation: Animation | undefined;

  function measureSelection(animate = false) {
    const button = group?.querySelector<HTMLButtonElement>(
      '[aria-pressed="true"]',
    );
    if (!button) return;
    // Layout sizes, as offsetLeft and offsetWidth round and leave the highlight
    // a pixel off its button. It stops short of the button's own separator.
    let left = 0;
    for (
      let other = button.previousElementSibling;
      other instanceof HTMLButtonElement;
      other = other.previousElementSibling
    )
      left += parseFloat(getComputedStyle(other).width);
    const { width, borderRightWidth } = getComputedStyle(button);
    const next = {
      left,
      width: parseFloat(width) - parseFloat(borderRightWidth),
    };
    // Choices that change without moving the highlight leave its slide running.
    const sliding = animation?.playState === 'running';
    if (
      sliding &&
      next.left === selection.left &&
      next.width === selection.width
    )
      return;
    const previous = highlight && getComputedStyle(highlight);
    // Where it shows now, part way through any slide.
    const from = previous && {
      left: selection.left + (parseFloat(previous.translate) || 0),
      width: selection.width * (parseFloat(previous.scale) || 1),
    };
    animation?.cancel();
    selection = next;
    // Transforms keep it sliding while the main thread is busy.
    if ((animate || sliding) && from && next.width)
      [animation] = playAnimations([
        {
          element: highlight,
          keyframes: [
            {
              translate: `${from.left - next.left}px`,
              scale: `${from.width / next.width} 1`,
            },
            { translate: '0px', scale: '1' },
          ],
        },
      ]);
  }
  $effect(() => {
    void value;
    void choices;
    untrack(() => measureSelection(requested === value));
    requested = undefined;
  });
  onMount(() => {
    const observer = new ResizeObserver(() => measureSelection());
    if (group) observer.observe(group);
    return () => {
      observer.disconnect();
      animation?.cancel();
    };
  });
</script>

<div
  bind:this={group}
  class={[
    'relative isolate inline-flex w-fit',
    resizeWithSidebar ? 'overflow-visible' : 'overflow-hidden',
    !compact && 'border border-line bg-surface',
  ]}
  data-sidebar-resize={resizeWithSidebar ? 'x' : undefined}
  data-sidebar-resize-origin={resizeWithSidebar ? '' : undefined}
  role="group"
  aria-label={ariaLabel}
>
  <span
    bind:this={highlight}
    aria-hidden="true"
    data-choice-selection
    class="pointer-events-none absolute inset-y-0 origin-left bg-accent-soft transition-none"
    style:left={`${selection.left}px`}
    style:width={`${selection.width}px`}
  ></span>
  {#each choices as choice (choice.value)}
    <button
      type="button"
      class={[
        'relative grid place-items-center text-xs transition-colors motion-reduce:transition-none focus-visible:-outline-offset-3',
        tall ? 'h-8.5' : 'h-8',
        compact
          ? 'w-(--compact-choice-width,2rem)'
          : 'min-w-11 border-r border-line px-2.5 last:border-r-0',
        value === choice.value
          ? 'text-accent'
          : 'text-muted hover:bg-surface-soft hover:text-foreground',
      ]}
      {disabled}
      aria-label={choice.count === undefined
        ? choice.label
        : `${choice.label} ${choice.count}`}
      title={choice.icon ? choice.label : undefined}
      aria-pressed={value === choice.value}
      onclick={() => {
        if (value !== choice.value) {
          requested = choice.value;
          onChange(choice.value);
        }
      }}
    >
      {#if choice.icon}
        <span data-sidebar-resize={resizeWithSidebar ? 'x-pos' : undefined}>
          <choice.icon class="size-3.5" />
        </span>
      {:else if choice.count === undefined}
        {choice.label}
      {:else}
        <span
          >{choice.label}<span
            class="ml-1.5 text-[10px] opacity-80 tabular-nums"
            >{choice.count}</span
          ></span
        >
      {/if}
    </button>
  {/each}
</div>
