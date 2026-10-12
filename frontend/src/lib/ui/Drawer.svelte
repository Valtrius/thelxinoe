<script lang="ts">
  import { onDestroy, type Snippet } from 'svelte';
  import type { HTMLDialogAttributes } from 'svelte/elements';
  import { twMerge } from 'tailwind-merge';
  import {
    cancelLayoutAnimations,
    playAnimations,
    type AnimationPlan,
  } from '../layout-animation';

  let {
    modal = false,
    side = 'left',
    open,
    onClose,
    children,
    class: className,
    ...rest
  }: Omit<HTMLDialogAttributes, 'children' | 'class' | 'open'> & {
    modal?: boolean;
    side?: 'left' | 'right';
    open: boolean;
    onClose: () => void;
    children: Snippet;
    class?: string;
  } = $props();
  let dialog: HTMLDialogElement;
  let opener: Element | null = null;
  let generation = 0;

  function closeDialog() {
    if (!dialog.open) return;
    dialog.close();
    if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
    opener = null;
  }

  $effect(() => {
    const opening = open;
    if (!opening && !dialog.open) return;
    const current = ++generation;
    const hidden = `translateX(${side === 'left' ? -100 : 100}%)`;
    const from = dialog.open ? getComputedStyle(dialog).transform : hidden;
    const backdrop = dialog.open
      ? getComputedStyle(dialog, '::backdrop').opacity
      : '0';
    cancelLayoutAnimations([dialog]);
    dialog.inert = !opening;
    if (opening && !dialog.open) {
      opener = document.activeElement;
      if (modal) dialog.showModal();
      else dialog.show();
    }
    const plans: AnimationPlan[] = [
      {
        element: dialog,
        keyframes: [
          { transform: from },
          { transform: opening ? 'none' : hidden },
        ],
        options: { fill: 'both' },
      },
    ];
    if (modal)
      plans.push({
        element: dialog,
        keyframes: [{ opacity: backdrop }, { opacity: opening ? 1 : 0 }],
        options: { pseudoElement: '::backdrop', fill: 'both' },
      });
    const animations = playAnimations(plans);
    const finish = () => {
      if (current !== generation) return;
      if (!opening) closeDialog();
      cancelLayoutAnimations([dialog]);
    };
    if (animations.length === 0) finish();
    else
      void Promise.allSettled(
        animations.map((animation) => animation.finished),
      ).then(finish);
  });

  onDestroy(() => {
    generation++;
    cancelLayoutAnimations([dialog]);
    closeDialog();
  });
</script>

<svelte:window
  onkeydown={(event) => {
    // Escape only cancels modal dialogs natively.
    if (open && !modal && event.key === 'Escape' && !event.defaultPrevented) {
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
