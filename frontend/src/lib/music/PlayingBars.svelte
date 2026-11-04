<script lang="ts">
  let { paused } = $props<{ paused: boolean }>();
</script>

<!-- The playing track's marker; its bars move while it plays. -->
<span
  class="inline-flex h-3 items-end gap-0.5 text-accent"
  role="img"
  aria-label={paused ? 'Paused' : 'Playing'}
>
  {#each [0.4, 1, 0.65] as height, index (index)}<i
      class={[
        'block w-0.75 origin-bottom bg-current',
        !paused && 'animate-music-bar motion-reduce:animate-none',
      ]}
      style:height={`${height * 100}%`}
      style:animation-delay={`${index * -0.35}s`}
      data-motion-indicator
    ></i>{/each}
</span>
