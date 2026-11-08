<script lang="ts">
  import { UserRound } from '@lucide/svelte';
  import { twMerge } from 'tailwind-merge';
  import { cover } from './types';
  let { src, class: className = '' } = $props<{
    src?: string | null;
    class?: string;
  }>();
  let failed = $state(false);
  const image = $derived(failed ? '' : cover(src));
</script>

<span
  class={twMerge(
    'grid shrink-0 place-items-end overflow-hidden border border-white/15 bg-surface-strong text-white/20',
    className,
  )}
  >{#if image}<img
      class="size-full object-cover"
      src={image}
      alt=""
      loading="lazy"
      onerror={() => (failed = true)}
    />{:else}<UserRound
      class="size-4/5 translate-y-[8%]"
      fill="currentColor"
      strokeWidth={0}
    />{/if}</span
>
