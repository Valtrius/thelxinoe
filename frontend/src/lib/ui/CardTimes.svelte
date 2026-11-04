<script lang="ts">
  import { formatClock } from '../providers/utils';

  let {
    duration,
    position = 0,
    showPosition,
    playing,
  }: {
    duration?: number | null;
    position?: number;
    showPosition: boolean;
    /** Counts down what is left instead of showing the full length. */
    playing: boolean;
  } = $props();
</script>

{#if duration}
  <span
    class="absolute right-2 bottom-3 border border-white/20 bg-black/75 px-2 py-1 font-mono text-[0.62rem] text-white phone-card:card-pixels phone-card:px-1.5 phone-card:py-0.75 phone-card:text-[9px]"
  >
    {playing
      ? `-${formatClock(Math.max(duration - position, 0))}`
      : formatClock(duration)}
  </span>
{/if}
{#if showPosition}
  <span
    class="absolute bottom-3 left-2 border border-accent/45 bg-black/75 px-2 py-1 font-mono text-[0.62rem] text-accent phone-card:card-pixels phone-card:px-1.5 phone-card:py-0.75 phone-card:text-[9px]"
  >
    {formatClock(position)}
  </span>
{/if}
