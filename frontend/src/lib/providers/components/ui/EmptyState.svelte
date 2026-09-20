<script lang="ts">
  import { RadioTower } from '@lucide/svelte';
  import Button from './Button.svelte';

  let {
    eyebrow = 'NO SIGNAL',
    title,
    message,
    actionLabel,
    onAction,
  }: {
    eyebrow?: string;
    title: string;
    message: string;
    actionLabel?: string | null;
    onAction?: (() => void) | null;
  } = $props();
</script>

<section
  class="panel grid min-h-72 place-items-center border border-(--line) bg-(--surface) p-8 text-center"
>
  <div class="max-w-md">
    <RadioTower class="mx-auto size-8 text-(--accent) opacity-70" />
    <p class="eyebrow mt-5">{eyebrow}</p>
    <h2 class="mt-2 text-xl tracking-[-0.035em]">{title}</h2>
    <p class="mt-3 text-sm leading-6 text-(--muted)">{message}</p>
    {#if actionLabel && onAction}
      <div class="mt-5"><Button onclick={onAction}>{actionLabel}</Button></div>
    {/if}
  </div>
</section>
