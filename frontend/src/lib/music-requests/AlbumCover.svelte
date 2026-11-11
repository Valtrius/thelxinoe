<script lang="ts">
  import { twMerge } from 'tailwind-merge';
  import { cover } from './types';
  let {
    src,
    title,
    artist,
    sizes,
    small = false,
    class: className = '',
  }: {
    src?: string | null;
    title: string;
    artist?: string | null;
    sizes?: string;
    /** A thumbnail too small for the names; the fallback shows the initial alone. */
    small?: boolean;
    class?: string;
  } = $props();
  let failed = $state(false);
  const image = $derived(failed ? '' : cover(src));
</script>

{#if image}<img
    class={twMerge('aspect-square object-cover', className)}
    src={image}
    {sizes}
    alt=""
    loading="lazy"
    onerror={() => (failed = true)}
  />{:else}<span
    class={twMerge(
      'flex aspect-square flex-col justify-between overflow-hidden bg-surface-strong',
      small ? 'items-center justify-center' : 'p-3',
      className,
    )}
    data-cover-fallback
    ><span
      class={[
        'leading-none font-bold tracking-tighter text-muted',
        small ? 'text-base' : 'text-4xl',
      ]}>{(artist || title).trim().charAt(0)}</span
    >{#if !small}<span class="min-w-0"
        >{#if artist}<span
            class="block truncate text-[8px] font-semibold tracking-[0.16em] text-muted uppercase"
            >{artist}</span
          >{/if}<span
          class="mt-0.5 line-clamp-2 block text-xs leading-tight font-bold text-foreground"
          >{title}</span
        ></span
      >{/if}</span
  >{/if}
