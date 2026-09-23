<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import { tv, type VariantProps } from 'tailwind-variants';

  const styles = tv({
    base: 'inline-flex items-center justify-center gap-2 border text-[0.68rem] font-semibold tracking-[0.08em] uppercase transition-colors disabled:pointer-events-none disabled:opacity-45',
    variants: {
      variant: {
        default:
          'border-line-strong bg-accent-soft text-accent hover:bg-accent/16',
        secondary:
          'border-line bg-surface-soft text-foreground hover:border-line-strong hover:text-accent',
        ghost:
          'border-transparent bg-transparent text-muted hover:bg-surface-soft hover:text-foreground',
        danger: 'border-danger/45 bg-danger/9 text-danger hover:bg-danger/16',
        player:
          'border-0 bg-transparent text-inherit hover:bg-[#ffffff24] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent data-[state=open]:bg-[#ffffff24]',
      },
      size: {
        default: 'h-9 px-3.5',
        sm: 'h-8 px-2.5 text-[0.62rem]',
        icon: 'size-9 p-0',
        form: 'min-h-8.5 px-3.25 py-2 disabled:pointer-events-auto disabled:opacity-46',
        'compact-icon':
          'inline-grid size-7.5 shrink-0 place-items-center border-0 p-0 disabled:pointer-events-auto disabled:opacity-46',
        credential:
          'h-4.5 shrink-0 gap-0.5 px-0.75 py-0 text-[9px] leading-none font-normal tracking-normal normal-case disabled:pointer-events-auto disabled:cursor-default [&_svg]:shrink-0',
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
