<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import { appearance, appearanceError, updateAppearance } from './appearance';
  import DensityControl from './ui/DensityControl.svelte';
  import ThemeControls from './ui/ThemeControls.svelte';
</script>

<div class="grid gap-5">
  <div class="flex flex-wrap items-center justify-between gap-4">
    <strong>Theme</strong>
    <ThemeControls />
  </div>
  <div class="flex flex-wrap items-center justify-between gap-4">
    <label for="animation-duration" class="font-semibold"
      >Animation duration</label
    >
    <div class="flex items-center gap-3">
      <input
        id="animation-duration"
        type="range"
        min="0"
        max="500"
        step="10"
        value={$appearance.animation_duration_ms}
        aria-valuetext={$appearance.animation_duration_ms
          ? `${$appearance.animation_duration_ms} ms`
          : 'No animation'}
        class="h-8 w-36 cursor-pointer accent-accent"
        oninput={(event) =>
          updateAppearance({
            animation_duration_ms: event.currentTarget.valueAsNumber,
          })}
      />
      <span aria-hidden="true" class="min-w-20 text-right text-xs tabular-nums">
        {$appearance.animation_duration_ms
          ? `${$appearance.animation_duration_ms} ms`
          : 'No animation'}
      </span>
    </div>
  </div>
  <div class="grid gap-5">
    <DensityControl
      label="Poster density"
      views="Movies, TV and Music"
      columns={$appearance.poster_columns}
      onChange={(poster_columns) => updateAppearance({ poster_columns })}
    />
    <DensityControl
      label="Video density"
      views="YouTube, Twitch and Kick"
      columns={$appearance.video_columns}
      onChange={(video_columns) => updateAppearance({ video_columns })}
    />
    <p class="m-0 text-muted">
      Hold Ctrl and scroll over a collection to change its density.
    </p>
  </div>
  <Switch
    checked={$appearance.fade_watched}
    onCheckedChange={(checked) => updateAppearance({ fade_watched: checked })}
    >Fade watched videos</Switch
  >
  {#if $appearanceError}<p role="status">{$appearanceError}</p>{/if}
</div>
