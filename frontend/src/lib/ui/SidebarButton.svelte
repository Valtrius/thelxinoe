<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import { tv, type VariantProps } from 'tailwind-variants';
  import NavigationItem from './NavigationItem.svelte';

  const styles = tv({
    base: 'flex w-full items-center whitespace-nowrap',
    variants: {
      size: {
        default: 'h-11 text-xs font-semibold tracking-[0.06em] uppercase',
        sm: 'h-8',
      },
    },
    defaultVariants: { size: 'default' },
  });

  const labelStyles = tv({
    base: 'min-w-0 flex-1 overflow-hidden text-left transition-opacity',
    variants: {
      collapsed: {
        true: 'opacity-0',
        false: 'opacity-100',
      },
      size: {
        default: '',
        sm: 'text-[0.62rem] uppercase',
      },
    },
    defaultVariants: { collapsed: false, size: 'default' },
  });

  type StyleProps = VariantProps<typeof styles>;
  type Props = Omit<HTMLButtonAttributes, 'children' | 'class'> &
    StyleProps & {
      children: Snippet;
      label: string;
      active?: boolean;
      collapsed?: boolean;
      resizeWithSidebar?: boolean;
      class?: string;
    };

  let {
    children,
    label,
    active = false,
    collapsed = false,
    resizeWithSidebar = true,
    size = 'default',
    class: className,
    type = 'button',
    ...rest
  }: Props = $props();
</script>

<NavigationItem
  {resizeWithSidebar}
  {type}
  {active}
  class={styles({ size, class: className })}
  {...rest}
>
  <span class="flex h-5 w-18 shrink-0 items-center justify-center">
    {@render children()}
  </span>
  <span class={labelStyles({ collapsed, size })}>{label}</span>
</NavigationItem>
