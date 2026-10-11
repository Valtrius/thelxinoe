<script lang="ts">
  import Button from './Button.svelte';
  let { label, views, columns, onChange } = $props<{
    label: 'Poster density' | 'Video density';
    views: string;
    columns: number;
    onChange: (columns: number) => void;
  }>();
  const cardType = $derived(label === 'Poster density' ? 'poster' : 'video');
</script>

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
    <Button
      variant="secondary"
      size="form"
      class="min-w-7.5 p-1.5"
      aria-label={`Larger ${cardType} cards`}
      disabled={columns <= 3}
      onclick={() => onChange(columns - 1)}>−</Button
    >
    <span aria-live="polite">{columns} columns</span>
    <Button
      variant="secondary"
      size="form"
      class="min-w-7.5 p-1.5"
      aria-label={`Smaller ${cardType} cards`}
      disabled={columns >= 12}
      onclick={() => onChange(columns + 1)}>+</Button
    >
  </div>
</div>
