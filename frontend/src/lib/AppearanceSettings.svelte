<script lang="ts">
  import { appearance, appearanceError, updateAppearance } from './appearance';
</script>

<section class="panel" aria-label="Appearance preferences">
  <p class="eyebrow">YOUR WORKSPACE</p>
  <h2>Appearance</h2>
  <p class="muted">
    Choose how your collection is displayed. Your preferences follow your
    account.
  </p>
  <div class="setting-row">
    <div>
      <strong>Media density</strong>
      <p class="muted">
        Hold Ctrl and scroll over a collection to change its card size.
      </p>
    </div>
    <div class="density-controls">
      <button
        class="secondary"
        aria-label="Larger media cards"
        disabled={$appearance.card_columns <= 3}
        onclick={() =>
          updateAppearance({ card_columns: $appearance.card_columns - 1 })}
        >−</button
      ><span>{$appearance.card_columns} columns</span><button
        class="secondary"
        aria-label="Smaller media cards"
        disabled={$appearance.card_columns >= 12}
        onclick={() =>
          updateAppearance({ card_columns: $appearance.card_columns + 1 })}
        >+</button
      >
    </div>
  </div>
  <label class="check-row"
    ><input
      type="checkbox"
      checked={$appearance.fade_watched}
      onchange={(e) =>
        updateAppearance({ fade_watched: e.currentTarget.checked })}
    />Fade watched videos</label
  >
  {#if $appearanceError}<p role="status">{$appearanceError}</p>{/if}
</section>
