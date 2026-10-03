<script lang="ts">
  import {
    providers,
    providerNames,
    saveProvider,
    type Provider,
  } from './availability';
  import Switch from '../ui/Switch.svelte';
  import Notice from '../ui/Notice.svelte';
  let { platform }: { platform: Provider } = $props();
  let enabled = $state(true);
  let busy = $state(false);
  let error = $state('');
  $effect(() => {
    if (!busy) enabled = $providers[platform];
  });
  async function save(checked: boolean) {
    enabled = checked;
    busy = true;
    error = '';
    try {
      await saveProvider(platform, checked);
    } catch (caught) {
      error = String(caught);
    } finally {
      enabled = $providers[platform];
      busy = false;
    }
  }
</script>

<div class="mb-4">
  <Switch
    checked={enabled}
    disabled={busy}
    onCheckedChange={(checked) => void save(checked)}
    >Enable {providerNames[platform]} integration</Switch
  >
  {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
</div>
