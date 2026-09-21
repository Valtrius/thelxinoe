<script lang="ts">
  import type { Snippet } from 'svelte';

  let { children, card = true }: { children: Snippet; card?: boolean } =
    $props();
</script>

<main
  class="auth-page flex h-dvh flex-col items-center justify-center gap-5.5 overflow-y-auto px-4 py-8 desktop-shell:mt-8 desktop-shell:h-[calc(100dvh-32px)]"
>
  {#if card}
    <div
      class="auth-card w-full max-w-112.5 border border-line bg-surface p-8 shadow-panel [&_h1]:my-3.5 [&_form]:mt-6.5"
    >
      {@render children()}
    </div>
  {:else}
    {@render children()}
  {/if}
</main>
