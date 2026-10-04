<script lang="ts">
  import Skeleton from './Skeleton.svelte';
  let {
    label,
    variant = 'rows',
    count = 4,
    class: className = '',
  } = $props<{
    label: string;
    variant?: 'rows' | 'settings' | 'statistics' | 'detail';
    count?: number;
    class?: string;
  }>();
</script>

<div
  role="status"
  aria-label={label}
  aria-busy="true"
  class={`min-w-0 ${className}`}
>
  <span class="sr-only">{label}</span>
  {#if variant === 'statistics'}
    <div class="grid min-w-0 gap-3">
      <div
        class="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-4"
        aria-hidden="true"
      >
        {#each Array.from({ length: 4 }, (_, index) => index) as index (index)}
          <div class="border border-line bg-surface p-4">
            <Skeleton class="w-24" />
            <Skeleton class="mt-3 h-6 w-32" />
          </div>
        {/each}
      </div>
      {#each Array.from({ length: 2 }, (_, index) => index) as index (index)}
        <div
          class="min-w-0 border border-line bg-surface p-4"
          aria-hidden="true"
        >
          <Skeleton class="mb-5 w-36" />
          <Skeleton class="h-60" />
        </div>
      {/each}
    </div>
  {:else if variant === 'detail'}
    <div
      class="flex min-h-95 items-end gap-7 border border-line bg-surface p-8 compact:min-h-80 compact:gap-4 compact:p-5"
      aria-hidden="true"
    >
      <Skeleton class="aspect-2/3 h-auto w-40 shrink-0 compact:hidden" />
      <div class="grid w-full max-w-190 min-w-0 gap-5">
        <Skeleton class="w-24" />
        <Skeleton class="h-9 w-3/4" />
        <Skeleton class="w-1/2" />
        <Skeleton class="h-16" />
        <Skeleton class="h-9 w-32" />
      </div>
    </div>
  {:else if variant === 'settings'}
    <div
      class="grid min-w-0 grid-cols-[repeat(auto-fit,minmax(min(100%,16rem),1fr))] gap-5"
      aria-hidden="true"
    >
      {#each Array.from({ length: count }, (_, index) => index) as index (index)}
        <div class="grid min-w-0 max-w-72 gap-2">
          <Skeleton class="w-24" />
          <Skeleton class="h-9" />
        </div>
      {/each}
    </div>
  {:else}
    <div class="grid min-w-0 divide-y divide-line" aria-hidden="true">
      {#each Array.from({ length: count }, (_, index) => index) as index (index)}
        <div class="flex min-w-0 items-center gap-4 py-4">
          <div class="grid min-w-0 flex-1 gap-2">
            <Skeleton class="max-w-64" />
            <Skeleton class="h-2 max-w-40" />
          </div>
          <Skeleton class="h-8 w-20 shrink-0" />
        </div>
      {/each}
    </div>
  {/if}
</div>
