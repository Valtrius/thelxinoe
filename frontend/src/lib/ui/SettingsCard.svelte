<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import { Check, LoaderCircle } from '@lucide/svelte';
  import { twMerge } from 'tailwind-merge';

  // A settings card: header (title, description, status, actions), a body of
  // SettingRow children or padded content, and an optional footer bar.
  let {
    title,
    description,
    children,
    actions,
    footer,
    badge,
    lead,
    tone = 'default',
    saving = false,
    autoSave = false,
    padded = false,
    class: className,
    ...rest
  }: Omit<HTMLAttributes<HTMLElement>, 'children' | 'class' | 'title'> & {
    title?: string;
    description?: string | Snippet;
    children: Snippet;
    actions?: Snippet;
    footer?: Snippet;
    badge?: Snippet;
    lead?: Snippet;
    tone?: 'default' | 'danger';
    saving?: boolean;
    autoSave?: boolean;
    padded?: boolean;
    class?: string;
  } = $props();
  const hasHeader = $derived(Boolean(title || description || actions));
</script>

<section
  class={twMerge(
    'panel settings-card min-w-0 scroll-mt-14 border border-line bg-surface',
    tone === 'danger' && 'border-danger/40',
    className,
  )}
  {...rest}
>
  {#if hasHeader}
    <header
      class="settings-card-head flex items-start justify-between gap-4 px-5.5 py-4 compact:flex-wrap compact:px-4"
    >
      {#if lead}{@render lead()}{/if}
      <div class="min-w-0 flex-1 basis-60">
        {#if title}
          <!-- The badge sits beside the heading so the card's name stays its title. -->
          <div class="flex flex-wrap items-center gap-2.5">
            <h2
              class={[
                'm-0 text-[15px] font-semibold tracking-[-0.01em]',
                tone === 'danger' && 'text-danger',
              ]}
            >
              {title}
            </h2>
            {#if badge}{@render badge()}{/if}
          </div>
        {/if}
        {#if typeof description === 'string'}
          <p class="mt-1 mb-0 text-xs leading-relaxed text-muted">
            {description}
          </p>
        {:else if description}
          {@render description()}
        {/if}
      </div>
      {#if actions || autoSave || saving}
        <div class="flex shrink-0 flex-wrap items-center gap-2 pt-0.5">
          {#if saving}
            <span
              class="inline-flex items-center gap-1.5 text-[11px] text-muted"
              role="status"
            >
              <LoaderCircle
                size={13}
                class="animate-spin motion-reduce:animate-none"
                aria-hidden="true"
              />Saving…
            </span>
          {:else if autoSave}
            <span
              class="inline-flex items-center gap-1.5 text-[11px] text-muted"
            >
              <Check size={13} class="text-success" aria-hidden="true" />Saves
              automatically
            </span>
          {/if}
          {#if actions}{@render actions()}{/if}
        </div>
      {/if}
    </header>
  {/if}
  <div
    class={[
      'settings-card-body min-w-0',
      hasHeader && 'border-t border-line',
      padded ? 'px-5.5 py-4 compact:px-4' : 'divide-y divide-line',
    ]}
  >
    {@render children()}
  </div>
  {#if footer}
    <footer
      class="settings-card-foot flex flex-wrap items-center justify-between gap-3 border-t border-line bg-surface-soft px-5.5 py-3 compact:px-4"
    >
      {@render footer()}
    </footer>
  {/if}
</section>
