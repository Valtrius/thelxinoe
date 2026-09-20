<script lang="ts">
  import { CircleCheck, CircleX, Info, TriangleAlert, X } from '@lucide/svelte';
  import { dismissToast, toasts } from '../../toasts';
  import Button from './Button.svelte';

  const toneClass: Record<string, string> = {
    info: 'text-(--accent) border-(--line-strong)',
    success:
      'text-(--success) border-[color-mix(in_srgb,var(--success)_35%,transparent)]',
    warning:
      'text-(--warning) border-[color-mix(in_srgb,var(--warning)_35%,transparent)]',
    error:
      'text-(--danger) border-[color-mix(in_srgb,var(--danger)_40%,transparent)]',
  };
</script>

<div
  class="pointer-events-none fixed right-4 bottom-4 z-100 grid max-h-[calc(100vh-5rem)] w-[min(26rem,calc(100vw-2rem))] gap-2 overflow-y-auto"
  aria-label="Notifications"
>
  {#each $toasts as toast (toast.id)}
    <section
      class={`panel pointer-events-auto flex max-w-full min-w-0 items-start gap-3 border bg-(--surface-strong) p-3 shadow-2xl ${toneClass[toast.tone]}`}
      role={toast.tone === 'error' || toast.tone === 'warning'
        ? 'alert'
        : 'status'}
    >
      {#if toast.tone === 'success'}
        <CircleCheck class="mt-0.5 size-4 shrink-0" />
      {:else if toast.tone === 'warning'}
        <TriangleAlert class="mt-0.5 size-4 shrink-0" />
      {:else if toast.tone === 'error'}
        <CircleX class="mt-0.5 size-4 shrink-0" />
      {:else}
        <Info class="mt-0.5 size-4 shrink-0" />
      {/if}
      <div class="min-w-0 flex-1">
        <h2 class="text-xs font-semibold tracking-[0.08em] uppercase">
          {toast.title}
        </h2>
        <p class="mt-1 text-xs leading-5 wrap-anywhere text-(--muted)">
          {toast.message}
        </p>
        {#if toast.detail}
          <details
            class="mt-2 max-w-full min-w-0 overflow-hidden text-[0.68rem] text-(--muted)"
          >
            <summary class="cursor-pointer">Technical details</summary>
            <p
              class="mt-2 max-w-full font-mono leading-5 wrap-anywhere whitespace-pre-wrap"
            >
              {toast.detail}
            </p>
          </details>
        {/if}
        {#if toast.actionLabel && toast.onAction}
          <div class="mt-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={toast.busy}
              onclick={toast.onAction}
            >
              {toast.actionLabel}
            </Button>
          </div>
        {/if}
        {#if toast.progress}
          <label class="mt-2 grid gap-1 text-xs text-(--muted)">
            {toast.progress.label}
            <progress
              class="h-2 w-full accent-(--accent)"
              value={toast.progress.value}
              max={toast.progress.max}
            ></progress>
          </label>
        {/if}
      </div>
      <button
        type="button"
        class="grid size-7 shrink-0 place-items-center text-(--muted) hover:bg-(--surface-soft) hover:text-(--foreground)"
        aria-label={`Close ${toast.title} notification`}
        disabled={toast.busy}
        onclick={() => dismissToast(toast.id)}
      >
        <X class="size-3.5" />
      </button>
    </section>
  {/each}
</div>
