<script lang="ts">
  import { TriangleAlert, X } from '@lucide/svelte';
  import Button from './Button.svelte';

  let {
    open,
    title,
    message,
    confirmLabel = 'Confirm',
    danger = false,
    eyebrow = 'CONFIRM / LOCAL DATA',
    busy = false,
    error = '',
    onConfirm,
    onCancel,
  }: {
    open: boolean;
    title: string;
    message: string;
    confirmLabel?: string;
    danger?: boolean;
    eyebrow?: string;
    busy?: boolean;
    error?: string;
    onConfirm: () => void;
    onCancel: () => void;
  } = $props();
  const id = $props.id();
  let dialog = $state<HTMLDialogElement>();

  $effect(() => {
    if (open && !dialog?.open) dialog?.showModal();
    else if (!open && dialog?.open) dialog.close();
  });
</script>

<dialog
  bind:this={dialog}
  class="m-0 h-full max-h-none w-full max-w-none border-0 bg-transparent p-0 text-(--foreground) backdrop:bg-black/65 backdrop:backdrop-blur-[2px]"
  aria-labelledby={id + '-title'}
  aria-describedby={id + '-description'}
  aria-busy={busy}
  onkeydown={(event) => {
    // Let the native modal handle Escape without dismissing a drawer behind it.
    if (event.key === 'Escape') event.stopPropagation();
  }}
  oncancel={(event) => {
    event.preventDefault();
    if (!busy) onCancel();
  }}
>
  <div class="grid min-h-full place-items-center p-5">
    <section
      class="panel relative z-10 w-full max-w-md border border-(--line-strong) bg-(--surface-strong) p-5 shadow-2xl"
    >
      <div class="flex items-start gap-3">
        <TriangleAlert
          class={`mt-1 size-5 shrink-0 ${danger ? 'text-(--danger)' : 'text-(--warning)'}`}
        />
        <div class="min-w-0 flex-1">
          <p class="eyebrow">{eyebrow}</p>
          <h2 id={id + '-title'} class="mt-2 text-lg">{title}</h2>
          <p
            id={id + '-description'}
            class="mt-2 text-sm leading-6 text-(--muted)"
          >
            {message}
          </p>
          {#if error}<p class="mt-3 text-sm text-(--danger)" role="alert">
              {error}
            </p>{/if}
        </div>
        <Button
          size="icon"
          variant="ghost"
          aria-label="Close"
          disabled={busy}
          onclick={onCancel}><X class="size-4" /></Button
        >
      </div>
      <div class="mt-6 flex flex-wrap justify-end gap-2">
        <Button variant="ghost" disabled={busy} onclick={onCancel}
          >Cancel</Button
        >
        <Button
          variant={danger ? 'danger' : 'default'}
          disabled={busy}
          onclick={onConfirm}>{confirmLabel}</Button
        >
      </div>
    </section>
    <button
      class="absolute inset-0"
      aria-label="Cancel confirmation"
      tabindex="-1"
      disabled={busy}
      onclick={onCancel}
    ></button>
  </div>
</dialog>
