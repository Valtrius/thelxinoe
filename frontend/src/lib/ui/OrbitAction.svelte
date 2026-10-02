<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
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
    details?: Snippet;
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
    details,
  }: Props = $props();
  const id = $props.id();
  let dismissed = $state(false);
  let keyboardFocus = $state(false);
  let hovered = $state(false);
  let button = $state<HTMLButtonElement>();
  let tooltip = $state<HTMLSpanElement>();
  let position = $state({ left: 0, top: 0, above: false });

  function placeTooltip() {
    if (!button || !tooltip) return;
    const anchor = button.getBoundingClientRect();
    const bounds = tooltip.getBoundingClientRect();
    const above = anchor.bottom + bounds.height > innerHeight - 8;
    position = {
      left: Math.max(
        8,
        Math.min(
          anchor.left + anchor.width / 2 - bounds.width / 2,
          innerWidth - bounds.width - 8,
        ),
      ),
      top: Math.max(8, above ? anchor.top - bounds.height : anchor.bottom),
      above,
    };
  }
  $effect(() => {
    void title;
    void description;
    if (!tooltip) return;
    if ((hovered || keyboardFocus) && !dismissed) {
      if (!tooltip.matches(':popover-open')) tooltip.showPopover();
      placeTooltip();
    } else if (tooltip.matches(':popover-open')) tooltip.hidePopover();
  });
  onMount(() => {
    const reposition = () => {
      if (tooltip?.matches(':popover-open')) placeTooltip();
    };
    const dismiss = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && tooltip?.matches(':popover-open'))
        dismissed = true;
    };
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    window.addEventListener('keydown', dismiss);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
      window.removeEventListener('keydown', dismiss);
    };
  });
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

<span
  role="group"
  class="relative inline-flex shrink-0"
  onpointerenter={() => {
    hovered = true;
    dismissed = false;
  }}
  onpointerleave={() => (hovered = false)}
  onfocusout={(event) => {
    if (
      !(event.relatedTarget instanceof Node) ||
      !event.currentTarget.contains(event.relatedTarget)
    )
      keyboardFocus = false;
  }}
>
  <button
    bind:this={button}
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
    bind:this={tooltip}
    {id}
    role="tooltip"
    popover="manual"
    class={`fixed inset-auto m-0 w-max max-w-[min(17rem,calc(100vw-3rem))] overflow-visible border-0 bg-transparent p-0 ${position.above ? 'pb-8' : 'pt-8'}`}
    style:left={`${position.left}px`}
    style:top={`${position.top}px`}
  >
    <span
      class="block border border-line-strong bg-surface-strong p-3 text-left shadow-lg"
    >
      <span
        class="block text-xs leading-5 wrap-anywhere font-medium tracking-normal text-foreground normal-case"
        >{title}</span
      >
      {#if description}<span
          class="mt-1 block text-[11px] leading-relaxed wrap-anywhere font-normal tracking-normal whitespace-pre-line text-muted normal-case"
          >{description}</span
        >{/if}
      {#if details}{@render details()}{/if}
    </span>
  </span>
  <span class="sr-only" role="status" aria-live="polite">{title}</span>
</span>
