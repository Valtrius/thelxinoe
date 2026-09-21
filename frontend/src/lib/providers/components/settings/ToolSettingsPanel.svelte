<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { FolderOpen, Package, Power, RefreshCw } from '@lucide/svelte';
  import {
    isPlugin,
    toolNames,
    type ToolView,
    type ToolId,
    type ToolSource,
    type MpvPreferences,
  } from '../../tools-api';
  import { toolsState } from '../../tools-state';
  import {
    activeToolPackage,
    displayToolPath,
    pluginSelection,
    toolOperationActive,
  } from '../../tools-presentation';
  import Button from '../../../ui/Button.svelte';
  import ToolSourceChoices from './ToolSourceChoices.svelte';
  import ToolVersions from './ToolVersions.svelte';

  let {
    tool,
    configurationSource,
    onConfigure,
    onImportConfiguration,
    onInstall,
  }: {
    tool: ToolView;
    configurationSource: MpvPreferences['source'];
    onConfigure: (file?: string) => void;
    onImportConfiguration: () => void;
    onInstall: (tool: ToolId, packageId: string) => void;
  } = $props();
  let choosing = $state(false);
  const plugin = $derived(isPlugin(tool.id));
  const active = $derived(activeToolPackage(tool));
  const mode = $derived(pluginSelection(tool));
  const pending = $derived($toolsState.pending[tool.id]);
  const checking = $derived($toolsState.checking.includes(tool.id));
  const operation = $derived(
    $toolsState.snapshot?.operation.tool === tool.id
      ? $toolsState.snapshot.operation
      : null,
  );
  const installing = $derived(
    toolOperationActive(operation) ||
      ['Installing...', 'Activating...', 'Repairing...'].includes(
        pending ?? '',
      ),
  );
  const preferencesDisabled = $derived(
    choosing ||
      checking ||
      pending === 'Removing...' ||
      pending === 'Activating...',
  );
  const disabled = $derived(preferencesDisabled || installing);
  const managed = $derived(tool.preference.source === 'managed');

  async function choosePath() {
    choosing = true;
    try {
      const path = await open({
        title: `Select ${toolNames[tool.id]} executable`,
        multiple: false,
        filters: [{ name: 'Windows executable', extensions: ['exe'] }],
      });
      if (typeof path === 'string')
        await toolsState.preference(tool.id, {
          source: 'custom',
          customPath: path,
        });
    } catch (error) {
      toolsState.report(tool.id, error);
    } finally {
      choosing = false;
    }
  }
  function chooseSource(source: ToolSource) {
    if (source === 'custom' && !tool.preference.customPath) void choosePath();
    else void toolsState.preference(tool.id, { source });
  }
</script>

<div class="grid gap-4">
  {#if plugin}
    {#if configurationSource !== 'managed'}
      <p class="text-xs text-muted">
        MPV uses the plugins in its {configurationSource === 'native'
          ? 'normal configuration'
          : 'selected configuration folder'}. Manage a copy in Thelxinoe to
        control plugins here. Your selected MPV executable stays the same.
      </p>
      <Button
        variant="ghost"
        size="sm"
        class="justify-self-start"
        onclick={() => onConfigure()}>Go to MPV configuration</Button
      >
    {/if}
    {#if !active && !tool.importedPaths.length && configurationSource === 'managed'}
      <p class="text-xs text-muted">
        No {toolNames[tool.id]} plugin is installed in this configuration yet. Download
        a version below or import your existing MPV configuration. Both work with
        your selected MPV executable.
      </p>
    {/if}
    <Button
      variant="secondary"
      size="sm"
      class="justify-self-start"
      disabled={preferencesDisabled ||
        Boolean($toolsState.pending.configuration)}
      onclick={onImportConfiguration}
      ><FolderOpen class="size-3.5" />Import existing MPV configuration...</Button
    >
    <fieldset
      disabled={preferencesDisabled || configurationSource !== 'managed'}
      class="grid gap-3 disabled:opacity-45"
    >
      <legend class="sr-only">Plugin selection</legend>
      <div class="flex flex-wrap gap-2">
        <button
          type="button"
          aria-pressed={mode === 'managed'}
          disabled={!active}
          title={!active
            ? 'Download a managed version first.'
            : 'Use the version downloaded by Thelxinoe'}
          class={`flex items-center gap-2 border px-3 py-2 text-left text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-45 ${mode === 'managed' ? 'border-line-strong bg-accent-soft' : 'border-line bg-surface-soft'}`}
          onclick={() => {
            if (mode !== 'managed')
              void toolsState.preference(tool.id, {
                source: 'managed',
                enabled: true,
              });
          }}><Package class="size-3.5" />Managed version</button
        >
        <button
          type="button"
          aria-pressed={mode === 'imported'}
          disabled={!tool.importedPaths.length}
          title={!tool.importedPaths.length
            ? 'Copy a configuration folder or add this plugin in MPV configuration first.'
            : 'Use the imported files without managed updates'}
          class={`flex items-center gap-2 border px-3 py-2 text-left text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-45 ${mode === 'imported' ? 'border-line-strong bg-accent-soft' : 'border-line bg-surface-soft'}`}
          onclick={() => {
            if (mode !== 'imported')
              void toolsState.preference(tool.id, {
                source: 'custom',
                enabled: true,
              });
          }}><FolderOpen class="size-3.5" />Imported local copy</button
        >
        <button
          type="button"
          aria-pressed={mode === 'off'}
          class={`flex items-center gap-2 border px-3 py-2 text-left text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-45 ${mode === 'off' ? 'border-line-strong bg-accent-soft' : 'border-line bg-surface-soft'}`}
          onclick={() => {
            if (mode !== 'off')
              void toolsState.preference(tool.id, { enabled: false });
          }}><Power class="size-3.5" />Off</button
        >
      </div>
      {#if mode === 'imported'}
        {#each tool.importedPaths as path (path)}<p
            class="font-mono text-xs wrap-anywhere text-muted"
          >
            {displayToolPath(path)}
          </p>{/each}
      {/if}
      {#if mode !== 'off'}<Button
          variant="ghost"
          size="sm"
          class="justify-self-start"
          onclick={() =>
            onConfigure(
              tool.id === 'sub-select'
                ? 'script-opts/sub_select.conf'
                : `script-opts/${tool.id}.conf`,
            )}>Edit plugin settings</Button
        >{/if}
      {#if !active && mode !== 'managed'}
        <div class="grid gap-3">
          <h3 class="text-xs font-semibold">
            Download and use a managed version
          </h3>
          <ToolVersions
            {tool}
            managed
            {disabled}
            {preferencesDisabled}
            {installing}
            {operation}
            {onInstall}
          />
          {#if !tool.versions.length}
            <Button
              variant="secondary"
              size="sm"
              class="justify-self-start"
              disabled={Boolean($toolsState.pending.updates)}
              onclick={() => void toolsState.checkUpdates()}
            >
              Check for plugin versions
            </Button>
          {/if}
        </div>
      {:else}
        <ToolVersions
          {tool}
          managed={mode === 'managed'}
          {disabled}
          {preferencesDisabled}
          {installing}
          {operation}
          {onInstall}
        />
      {/if}
    </fieldset>
  {:else}
    <ToolSourceChoices
      label={`${toolNames[tool.id]} executable`}
      value={tool.preference.source}
      disabled={preferencesDisabled}
      onChange={chooseSource}
    />
    <div class="grid gap-2 border border-line bg-surface-soft p-3">
      <div class="flex flex-wrap items-center gap-2">
        <p class="min-w-0 flex-1 font-mono text-xs wrap-anywhere text-muted">
          {managed && !active
            ? 'No managed version selected.'
            : displayToolPath(tool.diagnostic?.path ?? tool.selectedPath) ||
              'No resolved executable yet.'}
        </p>
        <div class="flex shrink-0 items-center gap-2">
          {#if tool.preference.source === 'custom'}<Button
              variant="secondary"
              size="sm"
              disabled={preferencesDisabled}
              onclick={choosePath}>Choose executable...</Button
            >{/if}
          <Button
            variant="ghost"
            size="sm"
            {disabled}
            onclick={() => void toolsState.check(tool.id)}
            ><RefreshCw
              class={`size-3.5 ${checking ? 'animate-spin motion-reduce:animate-none' : ''}`}
            />{checking ? 'Checking...' : 'Check executable'}</Button
          >
        </div>
      </div>
      {#if tool.diagnostic?.error}<p class="text-xs text-warning">
          {tool.diagnostic.error.message}
        </p>{/if}
      {#if tool.diagnostic?.warning}<p class="text-xs text-warning">
          {tool.diagnostic.warning}
        </p>{/if}
    </div>
    <ToolVersions
      {tool}
      {managed}
      {disabled}
      {preferencesDisabled}
      {installing}
      {operation}
      {onInstall}
    />
  {/if}
</div>
