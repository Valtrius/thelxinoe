<script lang="ts">
  import { Sun, Monitor, Moon } from '@lucide/svelte';
  import ExclusiveChoiceGroup from './ExclusiveChoiceGroup.svelte';
  import { appearance, updateAppearance } from '../appearance';
  let { resizeWithSidebar = false } = $props<{
    resizeWithSidebar?: boolean;
  }>();
  const choices = [
    { value: 'light', label: 'Light theme', icon: Sun },
    { value: 'system', label: 'System theme', icon: Monitor },
    { value: 'dark', label: 'Dark theme', icon: Moon },
  ] as const;
</script>

<ExclusiveChoiceGroup
  {choices}
  value={$appearance.theme}
  ariaLabel="Theme"
  compact
  {resizeWithSidebar}
  onChange={(theme) => updateAppearance({ theme })}
/>
