<script lang="ts">
  import { Check } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';

  let {
    watched,
    progress = null,
    label,
    busy = false,
    toggle,
  } = $props<{
    watched: boolean;
    /** How far an unfinished episode has been played, from 0 to 1. */
    progress?: number | null;
    label: string;
    busy?: boolean;
    toggle: () => void;
  }>();
</script>

<Button
  variant="ghost"
  size="compact-icon"
  class="group"
  aria-pressed={watched}
  aria-label={`Watched: ${label}`}
  title={watched ? 'Mark unwatched' : 'Mark watched'}
  disabled={busy}
  onclick={toggle}
>
  {#if watched}<span
      class="grid size-5 place-items-center rounded-full bg-accent text-background"
      ><Check size={12} strokeWidth={3} /></span
    >{:else if progress}<span
      class="size-5 rounded-full mask-[radial-gradient(circle,transparent_8.2px,#000_8.6px)] group-hover:opacity-80"
      style:background={`conic-gradient(var(--accent) 0 ${Math.round(progress * 100)}%, color-mix(in srgb, var(--muted) 45%, transparent) 0)`}
    ></span>{:else}<span
      class="grid size-5 place-items-center rounded-full border-[1.5px] border-muted/75 text-transparent group-hover:border-accent group-hover:text-accent/60"
      ><Check size={11} strokeWidth={3} /></span
    >{/if}
</Button>
