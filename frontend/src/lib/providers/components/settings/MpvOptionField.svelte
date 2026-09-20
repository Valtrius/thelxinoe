<script lang="ts">
  import { RotateCcw } from '@lucide/svelte';
  import type { MpvOption } from '../../tools-api';
  import Button from '../ui/Button.svelte';
  import { twMerge } from 'tailwind-merge';
  import { controlClass } from './settingsUi';

  let {
    option,
    value,
    disabled = false,
    requiresRaw = false,
    onChange,
  }: {
    option: MpvOption;
    value: string | undefined;
    disabled?: boolean;
    requiresRaw?: boolean;
    onChange: (value: string | null) => void;
  } = $props();
  const id = $props.id();
  const compactControlClass = twMerge(
    controlClass,
    'min-h-7 px-2 py-1 text-[0.68rem]',
  );
  const choices = $derived(
    option.type === 'Flag' ? ['yes', 'no'] : (option.choices ?? []),
  );
  const defaultValue = $derived(option['default-value']);
  const placeholder = $derived(
    defaultValue === undefined
      ? 'Default'
      : `Default: ${typeof defaultValue === 'string' ? defaultValue : JSON.stringify(defaultValue)}`,
  );
</script>

<div
  class="grid grid-cols-[minmax(0,1fr)_minmax(8rem,1fr)_auto] items-center gap-2 border-b border-(--line) px-2 py-1.5 last:border-b-0"
>
  <label for={id} class="text-[0.68rem] wrap-anywhere">
    <span class="font-mono">{option.name}</span>
    <span class="mt-0.5 block text-[0.6rem] text-(--muted)">
      {option.type}{option.min !== undefined
        ? ` · min ${option.min}`
        : ''}{option.max !== undefined ? ` · max ${option.max}` : ''}
    </span>
    {#if requiresRaw && !disabled}<span class="text-(--warning)"
        >Edit in raw mode</span
      >{/if}
  </label>
  {#if choices.length}
    <select
      {id}
      class={compactControlClass}
      value={value ?? ''}
      disabled={disabled || requiresRaw}
      onchange={(event) => onChange(event.currentTarget.value || null)}
    >
      <option value="">{placeholder}</option>
      {#each choices as choice (choice)}<option value={choice}>{choice}</option
        >{/each}
      {#if value && !choices.includes(value)}<option {value}>{value}</option
        >{/if}
    </select>
  {:else}
    <input
      {id}
      class={compactControlClass}
      type="text"
      value={value ?? ''}
      {placeholder}
      disabled={disabled || requiresRaw}
      onchange={(event) => onChange(event.currentTarget.value)}
    />
  {/if}
  <Button
    variant="secondary"
    size="sm"
    class="h-7 px-1.5"
    title={'Use default for ' + option.name}
    aria-label={'Reset ' + option.name}
    disabled={disabled || requiresRaw || value === undefined}
    onclick={() => onChange(null)}><RotateCcw class="size-3.5" /></Button
  >
</div>
