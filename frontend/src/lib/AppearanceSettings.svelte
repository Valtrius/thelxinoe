<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import { appearance, appearanceError, updateAppearance } from './appearance';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { eyebrowClass } from './ui/styles';
</script>

<Panel aria-label="Appearance preferences">
  <p class={eyebrowClass}>YOUR WORKSPACE</p>
  <div
    class="mb-4.5 flex items-center justify-between gap-4 py-4 [&_p]:mt-1.25 [&_p]:mb-0 compact:flex-wrap compact:items-start"
  >
    <div>
      <strong>Media density</strong>
      <p class="text-muted">
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
</Panel>
