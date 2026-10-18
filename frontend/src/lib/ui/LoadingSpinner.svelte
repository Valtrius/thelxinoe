<script lang="ts">
  import { LoaderCircle } from '@lucide/svelte';
  import { twMerge } from 'tailwind-merge';
  let {
    label,
    size = 20,
    class: className = '',
  } = $props<{ label: string; size?: number; class?: string }>();
</script>

<div
  role="status"
  aria-label={label}
  class={twMerge('flex items-center justify-center text-muted', className)}
>
  <LoaderCircle
    {size}
    class="animate-spin motion-reduce:animate-none"
    aria-hidden="true"
  />
  <span class="sr-only">{label}</span>
</div>
