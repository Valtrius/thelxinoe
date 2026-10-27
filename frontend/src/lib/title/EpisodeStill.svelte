<script lang="ts">
  import type { Snippet } from 'svelte';
  import { asset } from './catalog';

  let {
    stills,
    class: className = '',
    placeholder,
  } = $props<{
    /** Pictures of the episode, best first; the next is tried when one fails. */
    stills: string[];
    class?: string | (string | false | null | undefined)[];
    placeholder: Snippet;
  }>();
  // Seasons reload with new lists of the same pictures; only new pictures retry.
  const key = $derived(stills.join('\n'));
  let failed = $state<{ key: string; count: number }>({ key: '', count: 0 });
  const index = $derived(failed.key === key ? failed.count : 0);
  const source = $derived(stills[index] ? asset(stills[index]) : '');
</script>

{#if source}<img
    src={source}
    alt=""
    loading="lazy"
    decoding="async"
    class={['absolute inset-0 size-full object-cover', className]}
    onerror={() => (failed = { key, count: index + 1 })}
  />{:else}<div
    class="absolute inset-0 grid place-items-center content-center gap-1 bg-[repeating-linear-gradient(135deg,var(--surface-soft)_0_6px,transparent_6px_12px)] text-[10.5px] text-muted"
  >
    {@render placeholder()}
  </div>{/if}
