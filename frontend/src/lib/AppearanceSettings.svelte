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
