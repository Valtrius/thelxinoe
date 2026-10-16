<script lang="ts">
  import Button from './Button.svelte';
  // `row` renders only the stepper; the surrounding SettingRow shows the label.
  let {
    label,
    views,
    columns,
    onChange,
    variant = 'stack',
  } = $props<{
    label: 'Poster density' | 'Video density';
    views: string;
    columns: number;
    onChange: (columns: number) => void;
    variant?: 'stack' | 'row';
  }>();
  const cardType = $derived(label === 'Poster density' ? 'poster' : 'video');
</script>

{#snippet stepper()}
  <Button
    variant="secondary"
    size="form"
    class="min-w-7.5 p-1.5"
    aria-label={`Larger ${cardType} cards`}
    disabled={columns <= 3}
    onclick={() => onChange(columns - 1)}>−</Button
  >
  <span aria-live="polite" class="min-w-18 text-center">{columns} columns</span>
  <Button
    variant="secondary"
    size="form"
    class="min-w-7.5 p-1.5"
    aria-label={`Smaller ${cardType} cards`}
    disabled={columns >= 12}
    onclick={() => onChange(columns + 1)}>+</Button
  >
{/snippet}

{#if variant === 'row'}
  <div
    role="group"
    aria-label={label}
    class="flex items-center gap-2 text-[11px] whitespace-nowrap"
  >
    {@render stepper()}
  </div>
{:else}
  <div
    role="group"
    aria-label={label}
    class="flex flex-wrap items-center justify-between gap-4"
  >
    <div class="min-w-0 flex-1 basis-48">
      <strong>{label}</strong>
      <p class="mt-1.25 mb-0 text-muted">{views}</p>
    </div>
    <div class="flex items-center gap-2.5 text-[11px] whitespace-nowrap">
      {@render stepper()}
    </div>
  </div>
{/if}
