<script lang="ts">
  import {
    CalendarClock,
    CircleAlert,
    CircleCheck,
    CircleDashed,
    CircleSlash,
    Clock,
    Download,
  } from '@lucide/svelte';
  import ProgressBar from '../ui/ProgressBar.svelte';
  import { statusText, type TitleStatus } from './status';

  let {
    status,
    timezone,
    compact = false,
  } = $props<{
    status: TitleStatus;
    timezone: string;
    compact?: boolean;
  }>();
  const text = $derived(statusText(status, timezone));
  const icons = {
    available: CircleCheck,
    partial: CircleDashed,
    downloading: Download,
    queued: Clock,
    upcoming: CalendarClock,
    missing: CircleAlert,
    unmonitored: CircleSlash,
    unavailable: CircleSlash,
  };
  const Icon = $derived(icons[status.state as keyof typeof icons]);
  const toneClass = {
    success: 'text-success',
    accent: 'text-accent',
    warning: 'text-warning',
    muted: 'text-muted',
  };
</script>

<span
  class={[
    'inline-flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1',
    compact ? 'text-[11px]' : 'text-xs font-semibold',
  ]}
  data-title-status={status.state}
>
  <span class={['inline-flex items-center gap-1.5', toneClass[text.tone]]}
    ><Icon size={compact ? 13 : 15} class="shrink-0" />{text.label}</span
  >{#if status.state === 'downloading' && (status.progress ?? 0) < 1}<ProgressBar
      class="h-1 w-16 bg-line"
      value={(status.progress ?? 0) * 100}
      label={text.label}
    />{/if}{#if text.detail}<span class="font-normal text-muted"
      >{text.detail}</span
    >{/if}
</span>
