<script lang="ts">
  import type { Snippet } from 'svelte';
  import { X } from '@lucide/svelte';
  import Button from './Button.svelte';

  let {
    title,
    busy = false,
    onClose,
    children,
  } = $props<{
    title: string;
    busy?: boolean;
    onClose: () => void;
    children: Snippet;
  }>();
  const id = $props.id();
  let dialog: HTMLDialogElement;

  $effect(() => {
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
  aria-labelledby={id}
  aria-busy={busy}
  class="m-auto max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-2xl overflow-y-auto border border-line-strong bg-surface-strong p-5 text-foreground shadow-2xl backdrop:bg-black/65 backdrop:backdrop-blur-[2px] compact:p-4"
  onkeydown={(event) => {
    if (event.key === 'Escape') event.stopPropagation();
  }}
  oncancel={(event) => {
    event.preventDefault();
    if (!busy) onClose();
  }}
>
  <div class="mb-4 flex items-center justify-between gap-4">
    <h2 {id} class="text-base font-semibold">{title}</h2>
    <Button
      variant="ghost"
      size="icon"
      aria-label="Close"
      disabled={busy}
      onclick={onClose}
    >
      <X size={16} />
    </Button>
  </div>
  {@render children()}
</dialog>
