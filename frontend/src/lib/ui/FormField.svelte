<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLLabelAttributes } from 'svelte/elements';
  import { twMerge } from 'tailwind-merge';

  let {
    children,
    class: className,
    ...rest
  }: Omit<HTMLLabelAttributes, 'children' | 'class'> & {
    children: Snippet;
    class?: string;
  } = $props();
</script>

<!-- The snippet supplies the native control and its label association. -->
<label
  class={twMerge(
    'mb-4 flex flex-col gap-2 text-[11px] font-[550] has-[>input[type=checkbox]]:flex-row has-[>input[type=checkbox]]:items-center has-[>input[type=radio]]:flex-row has-[>input[type=radio]]:items-center',
    className,
  )}
  {...rest}
>
  {@render children()}
</label>
