<script lang="ts">
  import type { Snippet } from 'svelte';
  let {
    version,
    label,
    children,
    tools,
  }: { version: string; label: string; children: Snippet; tools?: Snippet } =
    $props();
</script>

<div aria-label={label} class="min-w-0">
  <div class="flex items-start justify-between gap-3">
    <div>
      <div class="flex h-9 items-center gap-2">
        <strong class="tabular-nums">{version}</strong>
        {@render children()}
      </div>
      <small>{label}</small>
    </div>
    {#if tools}<div class="flex h-9 shrink-0 items-center">
        {@render tools()}
      </div>{/if}
  </div>
</div>
