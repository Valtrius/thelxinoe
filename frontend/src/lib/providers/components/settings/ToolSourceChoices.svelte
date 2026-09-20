<script lang="ts">
  import { FolderOpen, Package } from '@lucide/svelte';
  import type { ToolSource } from '../../tools-api';
  import Button from '../ui/Button.svelte';
  let {
    value,
    label,
    disabled = false,
    onChange,
  }: {
    value: ToolSource;
    label: string;
    disabled?: boolean;
    onChange: (source: ToolSource) => void;
  } = $props();
</script>

<fieldset class="grid min-w-0 gap-3" {disabled}>
  <legend class="mb-2 text-xs font-semibold">{label}</legend>
  <div class="flex flex-wrap gap-2">
    {#each [{ managed: true, name: 'Managed by Thelxinoe', icon: Package }, { managed: false, name: 'Use my installation', icon: FolderOpen }] as choice (choice.managed)}
      <button
        type="button"
        aria-pressed={choice.managed === (value === 'managed')}
        onclick={() => {
          const next = choice.managed
            ? 'managed'
            : value === 'custom'
              ? 'custom'
              : 'system';
          if (next !== value) onChange(next);
        }}
        class={`flex cursor-pointer items-center gap-2 border px-3 py-2 text-left disabled:cursor-default disabled:opacity-45 ${choice.managed === (value === 'managed') ? 'border-(--line-strong) bg-(--accent-soft)' : 'border-(--line) bg-(--surface-soft)'}`}
      >
        <span class="flex items-center gap-2 text-xs font-semibold"
          ><choice.icon class="size-3.5" />{choice.name}</span
        >
      </button>
    {/each}
  </div>
  {#if value !== 'managed'}
    <div
      class="flex flex-wrap gap-2"
      role="group"
      aria-label="Find my executable"
    >
      <Button
        size="sm"
        variant={value === 'system' ? 'default' : 'secondary'}
        aria-pressed={value === 'system'}
        disabled={value === 'system'}
        class={value === 'system'
          ? 'disabled:cursor-default disabled:opacity-100'
          : ''}
        onclick={() => {
          if (value !== 'system') onChange('system');
        }}>Auto-detect</Button
      >
      <Button
        size="sm"
        variant={value === 'custom' ? 'default' : 'secondary'}
        aria-pressed={value === 'custom'}
        onclick={() => onChange('custom')}>Choose executable</Button
      >
    </div>
  {/if}
</fieldset>
