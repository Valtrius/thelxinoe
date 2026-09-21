<script lang="ts">
  import type { Snippet } from 'svelte';

  type SwitchSize = 'default' | 'sm';

  let {
    checked = $bindable(false),
    children,
    size = 'default',
    disabled = false,
    class: className = '',
    onCheckedChange,
  }: {
    checked: boolean;
    children: Snippet;
    size?: SwitchSize;
    disabled?: boolean;
    class?: string;
    onCheckedChange?: (checked: boolean) => void;
  } = $props();

  const trackClass = $derived(size === 'sm' ? 'h-4.5 w-8' : 'h-5.5 w-10');
  const knobClass = $derived(
    size === 'sm'
      ? 'top-1 left-1 size-2.5 peer-checked:translate-x-3.5'
      : 'top-1 left-1 size-3.5 peer-checked:translate-x-4.5',
  );

  function handleChange(event: Event) {
    const next = (event.currentTarget as HTMLInputElement).checked;
    if (onCheckedChange) onCheckedChange(next);
    else checked = next;
  }
</script>

<label
  class={`ui-switch inline-flex w-fit items-center gap-2 text-muted ${disabled ? 'cursor-not-allowed opacity-45' : 'cursor-pointer'} ${className}`}
>
  <span class={`relative shrink-0 ${trackClass}`}>
    <input
      class="peer absolute size-px opacity-0"
      type="checkbox"
      role="switch"
      {checked}
      {disabled}
      onchange={handleChange}
    />
    <span
      class="absolute inset-0 border border-line-strong bg-surface transition-colors duration-200 ease-out peer-checked:bg-accent-soft peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent motion-reduce:transition-none"
      aria-hidden="true"
    ></span>
    <span
      class={`pointer-events-none absolute bg-muted transition-[translate,background-color] duration-200 ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[translate] peer-checked:bg-accent motion-reduce:transition-none ${knobClass}`}
      aria-hidden="true"
    ></span>
  </span>
  <span>{@render children()}</span>
</label>
