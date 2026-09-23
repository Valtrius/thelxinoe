<script lang="ts" generics="Value extends string">
  import { onMount, type Component } from 'svelte';
  import type { IconProps } from '@lucide/svelte';

  type Choice = {
    value: Value;
    label: string;
    icon?: Component<Omit<IconProps, 'icon' | 'iconNode'>>;
  };

  let {
    choices,
    value,
    ariaLabel,
    compact = false,
    disabled = false,
    resizeWithSidebar = false,
    onChange,
  }: {
    choices: readonly Choice[];
    value: Value;
    ariaLabel: string;
    compact?: boolean;
    disabled?: boolean;
    resizeWithSidebar?: boolean;
    onChange: (value: Value) => void;
  } = $props();
  let group = $state<HTMLDivElement>();
  let selection = $state({ left: 0, width: 0 });

  function measureSelection() {
    const button = group?.querySelector<HTMLButtonElement>(
      '[aria-pressed="true"]',
    );
    if (button)
      selection = { left: button.offsetLeft, width: button.offsetWidth };
  }
  $effect(() => {
    void value;
    void choices;
    measureSelection();
  });
  onMount(() => {
    const observer = new ResizeObserver(measureSelection);
    if (group) observer.observe(group);
    return () => observer.disconnect();
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
    aria-hidden="true"
    data-choice-selection
    class="pointer-events-none absolute inset-y-0 left-0 bg-accent-soft transition-[translate,width] duration-200 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none"
    style:translate={`${selection.left}px 0`}
    style:width={`${selection.width}px`}
  ></span>
  {#each choices as choice (choice.value)}
    <button
      type="button"
      class={[
        'relative grid h-8 place-items-center text-xs transition-colors motion-reduce:transition-none focus-visible:-outline-offset-3',
        compact
          ? 'w-(--compact-choice-width,2rem)'
          : 'min-w-11 border-r border-line px-2.5 last:border-r-0',
        value === choice.value
          ? 'text-accent'
          : 'text-muted hover:bg-surface-soft hover:text-foreground',
      ]}
      {disabled}
      aria-label={choice.label}
      title={choice.icon ? choice.label : undefined}
      aria-pressed={value === choice.value}
      onclick={() => {
        if (value !== choice.value) onChange(choice.value);
      }}
    >
      {#if choice.icon}
        <span data-sidebar-resize={resizeWithSidebar ? 'x-pos' : undefined}>
          <choice.icon class="size-3.5" />
        </span>
      {:else}
        {choice.label}
      {/if}
    </button>
  {/each}
</div>
