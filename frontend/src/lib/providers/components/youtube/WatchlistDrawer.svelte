<script lang="ts">
  import { onMount, type Snippet } from 'svelte';

  let { onClose, children } = $props<{
    onClose: () => void;
    children: Snippet;
  }>();
  let dialog: HTMLDialogElement;

  onMount(() => {
    const opener = document.activeElement;
    dialog.show();
    return () => {
      dialog.close();
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
    };
  });
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === 'Escape' && !event.defaultPrevented) {
      event.preventDefault();
      onClose();
    }
  }}
/>

<dialog
  bind:this={dialog}
  aria-label="YouTube watchlists"
  class="fixed inset-x-0 top-(--mobile-player-height) z-60 m-0 h-[calc(100dvh-var(--mobile-player-height,0px))] max-h-none w-full max-w-none overflow-hidden border-0 bg-background p-0 text-foreground desktop-shell:mt-8 desktop-shell:h-[calc(100dvh-var(--mobile-player-height,0px)-32px)]"
  oncancel={(event) => {
    event.preventDefault();
    onClose();
  }}
>
  {@render children()}
</dialog>
