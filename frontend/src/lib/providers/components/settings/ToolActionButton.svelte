<script lang="ts">
  import type { ToolOperation } from '../../tools-api';
  import { toolOperationActive } from '../../tools-presentation';
  import { formatBytes } from '../../utils';
  import Button from '../ui/Button.svelte';

  let {
    label,
    pending = false,
    operation = null,
    disabled = false,
    onclick,
  }: {
    label: string;
    pending?: boolean;
    operation?: ToolOperation | null;
    disabled?: boolean;
    onclick: () => void;
  } = $props();
  const working = $derived(pending || toolOperationActive(operation));
  const percent = $derived(
    operation?.phase === 'downloading' && operation.total > 0
      ? Math.min(
          100,
          Math.max(
            0,
            Math.round((operation.downloaded / operation.total) * 100),
          ),
        )
      : undefined,
  );
  const progressLabel = $derived(
    operation?.phase === 'verifying'
      ? 'Installing…'
      : percent !== undefined
        ? `Downloading ${percent}%`
        : 'Preparing…',
  );
</script>

<Button
  variant="secondary"
  size="sm"
  class="relative min-w-32 overflow-hidden disabled:pointer-events-auto disabled:opacity-100"
  disabled={disabled || working}
  aria-busy={working}
  title={working && operation?.total
    ? `${formatBytes(operation.downloaded)} / ${formatBytes(operation.total)}`
    : label}
  {onclick}
>
  {#if working}
    <span
      class={`pointer-events-none absolute inset-y-0 left-0 bg-(--accent-soft) transition-[width] duration-150 ${percent === undefined ? 'animate-pulse motion-reduce:animate-none' : ''}`}
      style:width={`${percent ?? 100}%`}
      aria-hidden="true"
    ></span>
  {/if}
  <span class="relative grid">
    <span class="invisible col-start-1 row-start-1" aria-hidden="true"
      >{label}</span
    >
    <span class="col-start-1 row-start-1"
      >{working ? progressLabel : label}</span
    >
  </span>
</Button>
