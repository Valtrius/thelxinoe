<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import { tv, type VariantProps } from 'tailwind-variants';

  const styles = tv({
    base: 'wrap-anywhere',
    variants: {
      variant: {
        callout:
          'mb-3.25 border-l-3 bg-accent-soft px-3 py-2.5 text-[11px] [&_p]:my-0.75 [&_button]:mt-2',
        error:
          'mb-4 border border-danger/45 bg-danger/8 px-4 py-3 text-xs text-danger',
      },
      tone: {
        info: 'border-accent',
        warning: 'border-warning',
        danger: 'border-danger',
      },
    },
    defaultVariants: { variant: 'callout' },
  });
  let {
    children,
    variant = 'callout',
    tone,
    class: className,
    ...rest
  }: Omit<HTMLAttributes<HTMLDivElement>, 'children' | 'class'> &
    VariantProps<typeof styles> & {
      children: Snippet;
      class?: string;
    } = $props();
</script>

<div
  data-tone={tone ?? (variant === 'error' ? 'danger' : 'info')}
  class={styles({
    variant,
    tone: tone ?? (variant === 'callout' ? 'info' : undefined),
    class: className,
  })}
  {...rest}
>
  {@render children()}
</div>
