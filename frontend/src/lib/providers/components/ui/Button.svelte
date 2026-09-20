<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import { tv, type VariantProps } from 'tailwind-variants';

  const styles = tv({
    base: 'inline-flex items-center justify-center gap-2 border text-[0.68rem] font-semibold tracking-[0.08em] uppercase transition-colors disabled:pointer-events-none disabled:opacity-45',
    variants: {
      variant: {
        default:
          'border-(--line-strong) bg-(--accent-soft) text-(--accent) hover:bg-[color-mix(in_srgb,var(--accent)_16%,transparent)]',
        secondary:
          'border-(--line) bg-(--surface-soft) text-(--foreground) hover:border-(--line-strong) hover:text-(--accent)',
        ghost:
          'border-transparent bg-transparent text-(--muted) hover:bg-(--surface-soft) hover:text-(--foreground)',
        danger:
          'border-[color-mix(in_srgb,var(--danger)_45%,transparent)] bg-[color-mix(in_srgb,var(--danger)_9%,transparent)] text-(--danger) hover:bg-[color-mix(in_srgb,var(--danger)_16%,transparent)]',
      },
      size: {
        default: 'h-9 px-3.5',
        sm: 'h-8 px-2.5 text-[0.62rem]',
        icon: 'size-9 p-0',
      },
    },
    defaultVariants: { variant: 'default', size: 'default' },
  });

  type StyleProps = VariantProps<typeof styles>;
  type Props = Omit<HTMLButtonAttributes, 'class'> &
    StyleProps & { children: Snippet; class?: string };
  let {
    children,
    variant = 'default',
    size = 'default',
    class: className,
    type = 'button',
    ...rest
  }: Props = $props();
</script>

<button {type} class={styles({ variant, size, class: className })} {...rest}>
  {@render children()}
</button>
