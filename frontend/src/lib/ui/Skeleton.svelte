<script lang="ts">
  import { twMerge } from 'tailwind-merge';
  let { class: className = '' } = $props<{ class?: string }>();
</script>

<div
  data-skeleton
  aria-hidden="true"
  class={twMerge(
    'h-3 w-full animate-pulse bg-line motion-reduce:animate-none',
    className,
  )}
></div>
