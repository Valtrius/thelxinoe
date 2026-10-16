<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import { twMerge } from 'tailwind-merge';

  // One setting: label and help text on the left, the control on the right.
  // Pass `for` to label a native control by id, or read `labelId` from the
  // children snippet to point aria-labelledby at the row label.
  let {
    label,
    description,
    for: htmlFor,
    children,
    badge,
    lead,
    control = 'auto',
    sub = false,
    dimmed = false,
    class: className,
    ...rest
  }: Omit<HTMLAttributes<HTMLDivElement>, 'children' | 'class'> & {
    label: string;
    description?: string | Snippet;
    for?: string;
    children?: Snippet<[{ labelId: string }]>;
    badge?: Snippet;
    lead?: Snippet;
    control?: 'auto' | 'field' | 'block';
    sub?: boolean;
    dimmed?: boolean;
    class?: string;
  } = $props();
  const labelId = $props.id();
</script>

<div
  class={twMerge(
    'setting-row grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-8 gap-y-3 px-5.5 py-3.25 compact:grid-cols-1 compact:px-4',
    sub && 'bg-surface-soft pl-11 compact:pl-4',
    dimmed && 'opacity-55',
    control === 'block' && 'grid-cols-1',
    className,
  )}
  {...rest}
>
  <div class="flex min-w-0 items-center gap-3">
    {#if lead}{@render lead()}{/if}
    <div class="min-w-0">
      <div
        class="flex flex-wrap items-center gap-2 text-[13px] leading-snug font-[550]"
      >
        {#if htmlFor}
          <label for={htmlFor} id={labelId}>{label}</label>
        {:else}
          <span id={labelId}>{label}</span>
        {/if}
        {#if badge}{@render badge()}{/if}
      </div>
      {#if typeof description === 'string'}
        <p class="mt-0.75 mb-0 text-xs leading-relaxed text-muted">
          {description}
        </p>
      {:else if description}
        {@render description()}
      {/if}
    </div>
  </div>
  {#if children}
    <div
      class={[
        'setting-control flex min-w-0 items-center justify-end gap-2 compact:flex-wrap compact:justify-start',
        control === 'field' &&
          'w-72 max-w-full compact:w-full [&>input]:w-full [&>select]:w-full',
        control === 'block' && 'justify-start',
      ]}
    >
      {@render children({ labelId })}
    </div>
  {/if}
</div>
