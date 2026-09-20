<script lang="ts">
  import ToolsSettings from './providers/components/settings/ToolsSettings.svelte';
  let notice = $state('');
</script>

<div class="provider-surface min-w-0">
  {#if notice}<p class="mb-3 text-xs text-(--muted)" role="status">
      {notice}
    </p>{/if}
  <ToolsSettings onNotice={(message) => (notice = message)} />
</div>
