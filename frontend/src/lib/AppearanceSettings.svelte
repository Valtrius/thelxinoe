<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import { appearance, appearanceError, updateAppearance } from './appearance';
  import Button from './ui/Button.svelte';
  import ThemeControls from './ui/ThemeControls.svelte';
</script>

<div class="grid gap-5">
  <div class="flex flex-wrap items-center justify-between gap-4">
    <strong>Theme</strong>
    <ThemeControls />
  </div>
  <div class="flex flex-wrap items-center justify-between gap-4">
    <div class="min-w-0 flex-1 basis-48">
      <strong>Media density</strong>
      <p class="mt-1.25 mb-0 text-muted">
        Hold Ctrl and scroll over a collection to change its card size.
      </p>
    </div>
    <div class="flex items-center gap-2.5 text-[11px] whitespace-nowrap">
      <Button
        variant="secondary"
        size="form"
        class="min-w-7.5 p-1.5"
        aria-label="Larger media cards"
        disabled={$appearance.card_columns <= 3}
        onclick={() =>
          updateAppearance({ card_columns: $appearance.card_columns - 1 })}
        >−</Button
      ><span>{$appearance.card_columns} columns</span><Button
        variant="secondary"
        size="form"
        class="min-w-7.5 p-1.5"
        aria-label="Smaller media cards"
        disabled={$appearance.card_columns >= 12}
        onclick={() =>
          updateAppearance({ card_columns: $appearance.card_columns + 1 })}
        >+</Button
      >
    </div>
  </div>
  <Switch
    checked={$appearance.fade_watched}
    onCheckedChange={(checked) => updateAppearance({ fade_watched: checked })}
    >Fade watched videos</Switch
  >
  {#if $appearanceError}<p role="status">{$appearanceError}</p>{/if}
</div>
