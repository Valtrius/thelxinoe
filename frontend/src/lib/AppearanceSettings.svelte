<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import { appearance, appearanceError, updateAppearance } from './appearance';
  import DensityControl from './ui/DensityControl.svelte';
  import ThemeControls from './ui/ThemeControls.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  const ids = $props.id();
</script>

<SettingRow label="Theme">
  <ThemeControls />
</SettingRow>
<SettingRow
  label="Animation duration"
  description="Set to 0 ms to turn motion off."
  for="animation-duration"
>
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
    class="h-8 w-40 cursor-pointer accent-accent compact:w-full"
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
</SettingRow>
<SettingRow
  label="Poster density"
  description="Movies, TV and Music. Ctrl + scroll over a collection also changes it."
>
  <DensityControl
    variant="row"
    label="Poster density"
    views="Movies, TV and Music"
    columns={$appearance.poster_columns}
    onChange={(poster_columns) => updateAppearance({ poster_columns })}
  />
</SettingRow>
<SettingRow
  label="Video density"
  description="YouTube, Twitch and Kick. Ctrl + scroll over a collection also changes it."
>
  <DensityControl
    variant="row"
    label="Video density"
    views="YouTube, Twitch and Kick"
    columns={$appearance.video_columns}
    onChange={(video_columns) => updateAppearance({ video_columns })}
  />
</SettingRow>
<SettingRow
  label="Fade watched videos"
  description="Dim videos you have already finished."
  for={`${ids}-fade`}
>
  <Switch
    id={`${ids}-fade`}
    checked={$appearance.fade_watched}
    onCheckedChange={(checked) => updateAppearance({ fade_watched: checked })}
  />
</SettingRow>
{#if $appearanceError}<p
    role="status"
    class="m-0 px-5.5 py-3 text-xs text-muted compact:px-4"
  >
    {$appearanceError}
  </p>{/if}
