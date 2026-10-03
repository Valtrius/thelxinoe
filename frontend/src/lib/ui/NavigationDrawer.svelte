<script lang="ts">
  import { onMount, type Snippet } from 'svelte';

  let { onClose, children } = $props<{
    onClose: () => void;
    children: Snippet;
  }>();
  let dialog: HTMLDialogElement;

  onMount(() => {
    const opener = document.activeElement;
    dialog.showModal();
    return () => {
      dialog.close();
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
    };
  });
</script>

<dialog
  bind:this={dialog}
  id="mobile-navigation"
  aria-label="Navigation menu"
  class="inset-0 m-0 h-dvh max-h-none w-64 max-w-[calc(100vw-3rem)] overflow-hidden border-0 bg-background p-0 text-foreground backdrop:bg-black/53 desktop-shell:mt-8 desktop-shell:h-[calc(100dvh-32px)]"
  oncancel={(event) => {
    event.preventDefault();
    onClose();
  }}
  onkeydown={(event) => {
    if (event.key === 'Escape') event.stopPropagation();
  }}
  onclick={(event) => {
    if (event.target !== dialog) return;
    const bounds = dialog.getBoundingClientRect();
    if (
      event.clientX < bounds.left ||
      event.clientX > bounds.right ||
      event.clientY < bounds.top ||
      event.clientY > bounds.bottom
    )
      onClose();
  }}
>
  {@render children()}
</dialog>
