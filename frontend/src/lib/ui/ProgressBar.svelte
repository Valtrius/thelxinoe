<script lang="ts">
  import { twMerge } from 'tailwind-merge';

  let {
    value = null,
    indeterminate = false,
    label,
    class: className,
    barClass,
  }: {
    value?: number | null;
    indeterminate?: boolean;
    label?: string;
    class?: string;
    barClass?: string;
  } = $props();
  const percent = $derived(
    value === null || !Number.isFinite(value)
      ? null
      : Math.min(100, Math.max(0, value)),
  );
</script>

<span
  class={twMerge(
    'relative block h-1 bg-accent-soft text-accent',
    indeterminate && 'overflow-hidden',
    className,
  )}
  role={label ? 'progressbar' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : true}
  aria-valuemin={label ? 0 : undefined}
  aria-valuemax={label ? 100 : undefined}
  aria-valuenow={label && !indeterminate ? (percent ?? undefined) : undefined}
  aria-valuetext={label && !indeterminate
    ? percent === null
      ? 'Progress unavailable'
      : `${percent}%`
    : undefined}
>
  <span
    class={twMerge(
      'block h-full bg-current',
      indeterminate
        ? 'absolute -left-2/5 w-2/5 animate-progress-travel motion-reduce:left-[30%] motion-reduce:animate-none'
        : 'w-(--progress)',
      barClass,
    )}
    style:--progress={indeterminate ? undefined : `${percent ?? 0}%`}
  ></span>
</span>
