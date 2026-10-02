<script lang="ts">
  import type { Snippet } from 'svelte';
  import { X } from '@lucide/svelte';
  import Button from './Button.svelte';

  let {
    title,
    busy = false,
    onClose,
    children,
    size = 'default',
    expanded = false,
    actions,
    closeLabel = 'Close',
  } = $props<{
    title: string;
    busy?: boolean;
    onClose: () => void;
    children: Snippet;
    size?: 'default' | 'editor';
    expanded?: boolean;
    actions?: Snippet;
    closeLabel?: string;
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
  class="m-auto border border-line-strong bg-surface-strong text-foreground shadow-2xl backdrop:bg-black/65 backdrop:backdrop-blur-[2px] {size ===
  'editor'
    ? `h-[min(850px,calc(100dvh-2rem))] max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-7xl overflow-hidden p-0 open:flex open:flex-col compact:m-0 compact:h-dvh compact:max-h-dvh compact:w-full compact:max-w-none ${expanded ? 'm-0! h-dvh! max-h-dvh! w-full! max-w-none!' : ''}`
    : 'max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-2xl overflow-y-auto p-5 compact:p-4'}"
  onkeydown={(event) => {
    if (event.key === 'Escape') event.stopPropagation();
  }}
  oncancel={(event) => {
    event.preventDefault();
    if (!busy) onClose();
  }}
>
  <div
    class="flex shrink-0 items-center justify-between gap-4 {size === 'editor'
      ? 'border-b border-line px-4 py-3'
      : 'mb-4'}"
  >
    <h2 {id} class="text-base font-semibold">{title}</h2>
    <div class="flex items-center gap-1">
      {#if actions}{@render actions()}{/if}
      <Button
        variant="ghost"
        size="icon"
        aria-label={closeLabel}
        disabled={busy}
        onclick={onClose}
      >
        <X size={16} />
      </Button>
    </div>
  </div>
  {@render children()}
</dialog>
