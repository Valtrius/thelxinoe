<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { toolsApi, type ToolId } from '../../tools-api';
  import { toolsState } from '../../tools-state';
  import {
    executableSelectionKey,
    toolOperationActive,
  } from '../../tools-presentation';
  import Button from '../../../ui/Button.svelte';
  import InfoBubble from './InfoBubble.svelte';
  import ToolsInventory from './ToolsInventory.svelte';
  import ToolSettingsPanel from './ToolSettingsPanel.svelte';
  import MpvConfigEditor from './MpvConfigEditor.svelte';

  let { onNotice }: { onNotice: (message: string) => void } = $props();
  let expanded = $state<ToolId | null>(null);
  let configurationFile = $state('mpv.conf');
  let configurationRequest = $state(0);
  let configurationEditor = $state<{
    importConfiguration: () => Promise<void>;
  }>();
  const snapshot = $derived($toolsState.snapshot);

  function install(tool: ToolId, packageId: string) {
    const installed = snapshot?.tools
      .find((item) => item.id === tool)
      ?.installed.some((item) => item.package.id === packageId);
    void toolsState.run(
      tool,
      () =>
        installed
          ? toolsApi.activate(tool, packageId)
          : toolsApi.install(packageId),
      installed ? 'Activating...' : 'Installing...',
    );
  }
  async function configure(file = 'mpv.conf') {
    configurationFile = file;
    configurationRequest += 1;
    expanded = 'mpv';
    await tick();
    const editor = document.getElementById('mpv-configuration');
    editor?.scrollIntoView({ block: 'start', behavior: 'instant' });
    editor?.focus({ preventScroll: true });
  }
  async function importConfiguration() {
    await configure();
    await configurationEditor?.importConfiguration();
  }
  onMount(() => {
    if (!snapshot) void toolsState.refresh().catch(() => {});
  });
</script>

<section aria-label="Tools settings" class="grid min-w-0 gap-3">
  {#if $toolsState.loadError}<p class="text-xs text-warning" role="alert">
      {$toolsState.loadError}
      <button
        class="underline"
        onclick={() => void toolsState.refresh().catch(() => {})}
        >Retry reading tool settings</button
      >
    </p>{/if}
  {#if snapshot}
    <ToolsInventory
      {snapshot}
      {expanded}
      pending={$toolsState.pending}
      errors={$toolsState.errors}
      checking={$toolsState.checking}
      onToggle={(id) => (expanded = expanded === id ? null : id)}
      onInstall={install}
      onCheck={() => void toolsState.checkAll()}
      onUpdates={() => void toolsState.checkUpdates()}
    >
      {#snippet detail(tool)}
        {#if $toolsState.errors[tool.id]}<p
            role="alert"
            class="text-xs text-danger"
          >
            {$toolsState.errors[tool.id]}
          </p>{/if}
        <ToolSettingsPanel
          {tool}
          configurationSource={snapshot.mpv.source}
          onConfigure={configure}
          onImportConfiguration={importConfiguration}
          onInstall={install}
        />
        {#if tool.id === 'mpv'}
          {@const mpvExecutableKey = executableSelectionKey(tool)}
          <div
            id="mpv-configuration"
            tabindex="-1"
            class="border-t border-line pt-4 focus:outline-none"
          >
            <div class="mb-3 flex items-center gap-1">
              <h3 class="text-sm font-semibold">
                MPV configuration and scripts
              </h3>
              <InfoBubble label="MPV configuration information">
                <a
                  class="underline"
                  href="https://mpv.io/manual/stable/"
                  target="_blank"
                  rel="noreferrer">MPV configuration reference</a
                >
              </InfoBubble>
            </div>
            {#if $toolsState.errors.configuration}<p
                role="alert"
                class="mb-3 text-xs text-danger"
              >
                {$toolsState.errors.configuration}
              </p>{/if}
            <MpvConfigEditor
              bind:this={configurationEditor}
              preferences={snapshot.mpv}
              initialFile={configurationFile}
              request={configurationRequest}
              executableKey={mpvExecutableKey}
              directory={snapshot.directory}
              executableBusy={$toolsState.checking.includes('mpv') ||
                ['Installing...', 'Activating...', 'Repairing...'].includes(
                  $toolsState.pending.mpv ?? '',
                ) ||
                (snapshot.operation.tool === 'mpv' &&
                  toolOperationActive(snapshot.operation))}
              {onNotice}
              onError={(error) => toolsState.report('configuration', error)}
              onChanged={toolsState.refresh}
            />
          </div>
        {/if}
      {/snippet}
    </ToolsInventory>
    {#if $toolsState.errors.updates || snapshot.catalogError}<p
        class="text-xs text-warning"
        role="status"
      >
        {$toolsState.errors.updates || snapshot.catalogError}
      </p>{/if}
  {:else}
    <p class="text-xs text-muted" role="status">
      Reading saved tool settings...
    </p>
    <Button
      variant="secondary"
      size="sm"
      class="justify-self-start"
      onclick={() => void toolsState.refresh().catch(() => {})}>Retry</Button
    >
  {/if}
</section>
