<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import type { HTMLDialogAttributes } from 'svelte/elements';
  import { twMerge } from 'tailwind-merge';

  let {
    modal = false,
    onClose,
    children,
    class: className,
    ...rest
  }: Omit<HTMLDialogAttributes, 'children' | 'class' | 'open'> & {
    modal?: boolean;
    onClose: () => void;
    children: Snippet;
    class?: string;
  } = $props();
  let dialog: HTMLDialogElement;

  onMount(() => {
    const opener = document.activeElement;
    if (modal) dialog.showModal();
    else dialog.show();
    return () => {
      dialog.close();
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
    };
  });
</script>

<svelte:window
  onkeydown={(event) => {
    // Escape only cancels modal dialogs natively.
    if (!modal && event.key === 'Escape' && !event.defaultPrevented) {
      event.preventDefault();
      onClose();
    }
  }}
/>

<!-- Drawers cover the page, so sidebar motion never animates their contents. -->
<dialog
  bind:this={dialog}
  data-sidebar-group
  class={twMerge(
    'm-0 max-h-none overflow-hidden border-0 bg-background p-0 text-foreground desktop-shell:mt-8',
    className,
  )}
  {...rest}
  oncancel={(event) => {
    event.preventDefault();
    onClose();
  }}
  onkeydown={(event) => {
    if (modal && event.key === 'Escape') event.stopPropagation();
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
