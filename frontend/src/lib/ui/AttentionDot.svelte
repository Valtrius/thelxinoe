<script lang="ts">
  import { attentionDescription, type AttentionItem } from '../attention';
  import type { ActivityItem } from '../activity';
  import { twMerge } from 'tailwind-merge';
  let {
    items = [],
    activity = [],
    class: className = '',
  } = $props<{
    items?: AttentionItem[];
    activity?: ActivityItem[];
    class?: string;
  }>();
  const severity = $derived(
    items.some((item: AttentionItem) => item.severity === 'error')
      ? 'error'
      : items.some((item: AttentionItem) => item.severity === 'warning')
        ? 'warning'
        : 'info',
  );
  const state = $derived(
    activity.some((item: ActivityItem) => item.state === 'running')
      ? 'running'
      : activity.some((item: ActivityItem) => item.state === 'unknown')
        ? 'unknown'
        : 'queued',
  );
  const description = $derived(attentionDescription(items, activity));
</script>

{#if activity.length}
  <span
    data-activity-state={state}
    data-attention-severity={items.length ? severity : undefined}
    role="img"
    aria-label={description}
    title={description}
    class={twMerge(
      'relative inline-grid size-3.5 shrink-0 place-items-center text-accent',
      className,
    )}
  >
    <svg
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      class={[
        'absolute inset-0 size-full',
        state === 'running'
          ? 'animate-spin [animation-duration:1.6s] motion-reduce:animate-none'
          : 'text-muted',
      ]}
    >
      <circle
        cx="8"
        cy="8"
        r="6"
        stroke="currentColor"
        stroke-width="1.2"
        class="opacity-20"
      />
      <circle
        cx="8"
        cy="8"
        r="6"
        stroke="currentColor"
        stroke-width="1.2"
        stroke-linecap="round"
        stroke-dasharray={state === 'running' ? '12 26' : undefined}
      />
    </svg>
    {#if items.length}<span
        class={[
          'size-1.75 rounded-full',
          severity === 'error'
            ? 'bg-danger'
            : severity === 'warning'
              ? 'bg-warning'
              : 'bg-accent',
        ]}
      ></span>{/if}
  </span>
{:else if items.length}
  <span
    data-attention-severity={severity}
    role="img"
    aria-label={attentionDescription(items)}
    title={attentionDescription(items)}
    class={[
      'inline-block size-1.75 shrink-0 rounded-full',
      severity === 'error'
        ? 'bg-danger'
        : severity === 'warning'
          ? 'bg-warning'
          : 'bg-accent',
      className,
    ]}
  ></span>
{/if}
