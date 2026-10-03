<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import { tv } from 'tailwind-variants';

  const styles = tv({
    base: 'group/nav-item relative isolate min-w-0 text-left text-muted transition-colors hover:text-foreground focus-visible:-outline-offset-3',
    variants: {
      active: {
        true: 'text-accent hover:text-accent',
      },
    },
  });

  let {
    active = false,
    resizeWithSidebar = false,
    description = '',
    children,
    class: className,
    type = 'button',
    href,
    ...rest
  }: Omit<HTMLButtonAttributes, 'children' | 'class'> & {
    active?: boolean;
    resizeWithSidebar?: boolean;
    description?: string;
    children: Snippet;
    class?: string;
    href?: string;
  } = $props();
  const descriptionId = $props.id();
</script>

<svelte:element
  this={href ? 'a' : 'button'}
  type={href ? undefined : type}
  {href}
  aria-current={active ? 'page' : undefined}
  class={styles({ active, class: className })}
  {...rest}
  aria-describedby={description ? descriptionId : rest['aria-describedby']}
>
  {#if !active}
    <span
      aria-hidden="true"
      data-sidebar-resize={resizeWithSidebar ? 'x' : undefined}
      class="pointer-events-none absolute inset-0 -z-10 bg-surface-soft opacity-0 group-hover/nav-item:opacity-100"
    ></span>
  {/if}
  {#if active}
    <span
      aria-hidden="true"
      data-sidebar-resize={resizeWithSidebar ? 'x' : undefined}
      class="pointer-events-none absolute inset-0 -z-10 bg-accent-soft"
    ></span>
    <span
      aria-hidden="true"
      data-sidebar-resize={resizeWithSidebar ? 'x-pos' : undefined}
      class="pointer-events-none absolute inset-y-0 right-0 z-0 w-0.75 bg-accent"
      data-nav-accent
    ></span>
  {/if}
  {@render children()}
  {#if description}<span id={descriptionId} class="sr-only">{description}</span
    >{/if}
</svelte:element>
