<script lang="ts">
  import type { Snippet } from 'svelte';
  type Props = {
    label: string;
    title: string;
    description?: string;
    progress?: number | null;
    tone?: 'muted' | 'accent' | 'success' | 'warning' | 'danger';
    motion?: 'none' | 'available' | 'working' | 'restoring';
    busy?: boolean;
    onclick: () => void;
    children: Snippet;
  };
  let {
    label,
    title,
    description = '',
    progress = null,
    tone = 'muted',
    motion = 'none',
    busy = false,
    onclick,
    children,
  }: Props = $props();
  const id = $props.id();
  let dismissed = $state(false);
  let keyboardFocus = $state(false);
  const percent = $derived(
    progress === null ? null : Math.max(0, Math.min(100, progress)),
  );
  const tones = {
    muted: 'text-muted',
    accent: 'text-accent',
    success: 'text-success',
    warning: 'text-warning',
    danger: 'text-danger',
  };
</script>

<span class="group/orbit relative inline-flex shrink-0">
  <button
    type="button"
    aria-label={label}
    aria-describedby={id}
    aria-disabled={busy}
    aria-busy={busy}
    class={`grid size-8 shrink-0 place-items-center rounded-sm border-0 bg-transparent p-0 hover:bg-accent-soft focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent ${tones[tone]}`}
    onpointerenter={() => (dismissed = false)}
    onpointerdown={() => (keyboardFocus = false)}
    onfocus={(event) => {
      dismissed = false;
      keyboardFocus = event.currentTarget.matches(':focus-visible');
    }}
    onblur={() => (keyboardFocus = false)}
    onkeydown={(event) => {
      if (event.key === 'Escape') {
        dismissed = true;
        event.stopPropagation();
      }
    }}
    onclick={() => {
      if (!busy) onclick();
    }}
  >
    <span class="relative grid size-7 place-items-center" aria-hidden="true">
      {#if motion !== 'none' || percent !== null}
        <svg
          viewBox="0 0 28 28"
          fill="none"
          stroke="currentColor"
          stroke-width="1.2"
          class="absolute inset-0 size-7"
        >
          <circle cx="14" cy="14" r="12" class="opacity-20" />
          <circle
            cx="14"
            cy="14"
            r="12"
            stroke-linecap="round"
            pathLength="100"
            stroke-dasharray={percent !== null
              ? `${percent} 100`
              : motion === 'available'
                ? '18 82'
                : '32 68'}
            class={percent !== null
              ? 'origin-center -rotate-90 transition-[stroke-dasharray] duration-300 motion-reduce:transition-none'
              : `origin-center animate-spin motion-reduce:animate-none ${motion === 'available' ? '[animation-duration:6s]' : '[animation-duration:1.25s]'} ${motion === 'restoring' ? '[animation-direction:reverse]' : ''}`}
          />
        </svg>
      {/if}
      <span
        class={motion === 'available'
          ? 'inline-flex animate-update-nudge motion-reduce:animate-none'
          : 'inline-flex'}>{@render children()}</span
      >
    </span>
  </button>
  {#if percent !== null}<span
      class="sr-only"
      role="progressbar"
      aria-label="Update download"
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={Math.floor(percent)}
    ></span>{/if}
  <span
    {id}
    role="tooltip"
    class={`pointer-events-none absolute top-full left-1/2 z-50 w-max max-w-[min(17rem,calc(100vw-3rem))] -translate-x-1/2 pt-8 opacity-0 transition-opacity group-hover/orbit:pointer-events-auto group-hover/orbit:opacity-100 motion-reduce:transition-none ${keyboardFocus ? 'pointer-events-auto opacity-100' : ''} ${dismissed ? 'hidden' : ''}`}
  >
    <span
      class="block border border-line-strong bg-surface-strong p-3 text-left shadow-lg"
    >
      <span
        class="block text-xs leading-5 font-medium tracking-normal text-foreground normal-case"
        >{title}</span
      >
      {#if description}<span
          class="mt-1 block text-[11px] leading-relaxed font-normal tracking-normal whitespace-pre-line text-muted normal-case"
          >{description}</span
        >{/if}
    </span>
  </span>
  <span class="sr-only" role="status" aria-live="polite">{title}</span>
</span>
