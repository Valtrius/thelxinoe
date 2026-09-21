<script lang="ts" generics="Value extends string">
  import type { Component } from 'svelte';
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

  function choiceClass(choice: Value) {
    const base = `grid h-8 place-items-center text-xs transition-colors focus-visible:-outline-offset-3 ${compact ? 'w-[var(--compact-choice-width,2rem)]' : 'min-w-11 border-r border-(--line) px-2.5 last:border-r-0'}`;
    return value === choice
      ? base + ' bg-(--accent-soft) text-(--accent)'
      : base +
          ' text-(--muted) hover:bg-(--surface-soft) hover:text-(--foreground)';
  }
</script>

<div
  class={`inline-flex ${resizeWithSidebar ? 'overflow-visible' : 'overflow-hidden'} ${compact ? '' : 'border border-(--line) bg-(--surface)'}`}
  data-sidebar-resize={resizeWithSidebar ? 'x' : undefined}
  data-sidebar-resize-origin={resizeWithSidebar ? '' : undefined}
  role="group"
  aria-label={ariaLabel}
>
  {#each choices as choice (choice.value)}
    <button
      type="button"
      class={choiceClass(choice.value)}
      {disabled}
      aria-label={choice.label}
      title={choice.icon ? choice.label : undefined}
      aria-pressed={value === choice.value}
      onclick={() => onChange(choice.value)}
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
