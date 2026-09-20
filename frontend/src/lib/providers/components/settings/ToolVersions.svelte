<script lang="ts">
  import {
    toolsApi,
    type ToolView,
    type ToolId,
    type ToolOperation,
  } from '../../tools-api';
  import {
    activeToolPackage,
    availableToolUpdate,
  } from '../../tools-presentation';
  import { Bell, Download, RefreshCw } from '@lucide/svelte';
  import { toolsState } from '../../tools-state';
  import { formatBytes } from '../../utils';
  import Button from '../ui/Button.svelte';
  import Switch from '../ui/Switch.svelte';
  import InfoBubble from './InfoBubble.svelte';
  import ToolActionButton from './ToolActionButton.svelte';
  import { controlClass } from './settingsUi';

  let {
    tool,
    managed,
    disabled = false,
    preferencesDisabled = false,
    installing = false,
    operation = null,
    onInstall,
  }: {
    tool: ToolView;
    managed: boolean;
    disabled?: boolean;
    preferencesDisabled?: boolean;
    installing?: boolean;
    operation?: ToolOperation | null;
    onInstall: (tool: ToolId, packageId: string) => void;
  } = $props();
  let selected = $state('');
  const active = $derived(activeToolPackage(tool));
  const update = $derived(availableToolUpdate(tool));
  const versions = $derived([
    ...tool.versions,
    ...tool.installed
      .map((item) => ({ ...item.package, recommended: false }))
      .filter(
        (item) => !tool.versions.some((version) => version.id === item.id),
      ),
  ]);
  const packageId = $derived(
    versions.some((version) => version.id === selected)
      ? selected
      : (update?.id ??
          tool.preference.active ??
          versions.find(
            (version) =>
              version.recommended &&
              version.channel === tool.preference.channel,
          )?.id ??
          ''),
  );
  const chosen = $derived(versions.find((version) => version.id === packageId));
  const installed = $derived(
    tool.installed.some((item) => item.package.id === packageId),
  );
  const updatePolicies = [
    { value: 'automatic' as const, name: 'Automatic', icon: Download },
    { value: 'notify' as const, name: 'Notify me', icon: Bell },
    { value: 'manual' as const, name: 'Manual', icon: RefreshCw },
  ];
  const id = $props.id();
</script>

<div class="grid gap-3">
  {#if managed}
    <div class="flex flex-wrap items-end gap-2">
      <div class="grid w-full max-w-sm gap-2 text-xs">
        <div class="flex items-center gap-1">
          <label for={id + '-managed-version'}>Managed version</label>
          {#if chosen}
            <InfoBubble label="Managed version information">
              <div class="grid gap-1.5">
                <a
                  href={chosen.homepage}
                  target="_blank"
                  rel="noreferrer"
                  class="underline">{chosen.provider}</a
                >
                <a
                  href={chosen.sourceUrl}
                  target="_blank"
                  rel="noreferrer"
                  class="underline">{chosen.license} · source</a
                >
              </div>
            </InfoBubble>
          {/if}
        </div>
        <select
          id={id + '-managed-version'}
          class={controlClass}
          value={packageId}
          disabled={disabled || !versions.length}
          onchange={(event) => (selected = event.currentTarget.value)}
        >
          {#if !versions.length}<option value=""
              >No upstream versions available yet</option
            >{/if}
          {#each versions as version (version.id)}
            <option value={version.id}
              >{version.version}{version.id === tool.preference.active
                ? ' · selected'
                : version.recommended
                  ? ' · latest upstream'
                  : ''}{tool.preference.heldVersions.includes(version.id)
                ? ' · held after rollback'
                : ''}</option
            >
          {/each}
        </select>
      </div>
      {#if chosen && packageId !== tool.preference.active}
        <ToolActionButton
          label={installed
            ? 'Use this version'
            : `Download and use · ${formatBytes(chosen.size)}`}
          {disabled}
          pending={installing}
          {operation}
          onclick={() => onInstall(tool.id, packageId)}
        />
      {/if}
    </div>
    {#if active}
      <div class="grid gap-3">
        <div
          class="flex flex-wrap gap-2"
          aria-label="When updates are available"
        >
          {#each updatePolicies as policy (policy.value)}
            <button
              type="button"
              disabled={preferencesDisabled}
              aria-pressed={tool.preference.updatePolicy === policy.value}
              onclick={() => {
                if (tool.preference.updatePolicy !== policy.value)
                  void toolsState.preference(tool.id, {
                    updatePolicy: policy.value,
                  });
              }}
              class={`flex cursor-pointer items-center gap-2 border px-3 py-2 text-left disabled:cursor-default disabled:opacity-45 ${tool.preference.updatePolicy === policy.value ? 'border-(--line-strong) bg-(--accent-soft)' : 'border-(--line) bg-(--surface-soft)'}`}
            >
              <span class="flex items-center gap-2 text-xs font-semibold"
                ><policy.icon class="size-3.5" />{policy.name}</span
              >
            </button>
          {/each}
        </div>
        <Switch
          size="sm"
          class="min-h-9 text-xs"
          checked={tool.preference.pinned}
          disabled={preferencesDisabled}
          onCheckedChange={(pinned) =>
            void toolsState.preference(tool.id, { pinned })}
          >Pin selected version</Switch
        >
      </div>
    {/if}
  {/if}
  {#if tool.installed.length}
    <details class="border-t border-(--line) pt-3">
      <summary class="cursor-pointer text-xs text-(--muted)"
        >Maintenance and downloaded versions · {tool.installed.length}</summary
      >
      <div class="mt-3 grid gap-3">
        {#each tool.installed as item (item.package.id)}
          {@const inUse = tool.inUse.includes(item.package.id)}
          {@const retained =
            item.package.id === tool.preference.active ||
            item.package.id === tool.preference.previous}
          <div
            class="flex flex-wrap items-center justify-between gap-2 border-t border-(--line) pt-3 text-xs"
          >
            <div>
              <span class="font-mono">{item.package.version}</span><span
                class="ml-2 text-(--muted)"
                >{item.package.id === tool.preference.active
                  ? managed
                    ? 'Selected'
                    : 'Retained selection'
                  : item.package.id === tool.preference.previous
                    ? 'Previous'
                    : 'Downloaded'}{inUse ? ' · in use' : ''}</span
              >
            </div>
            <div class="flex items-center gap-2">
              {#if managed && item.package.id === tool.preference.previous}<Button
                  variant="ghost"
                  size="sm"
                  {disabled}
                  onclick={() =>
                    void toolsState.run(
                      tool.id,
                      () => toolsApi.rollback(tool.id),
                      'Activating...',
                    )}>Rollback</Button
                >{/if}
              {#if managed}<Button
                  variant="ghost"
                  size="sm"
                  disabled={disabled || inUse}
                  title={inUse
                    ? 'Finish players and downloads using this version before repairing it.'
                    : 'Download a verified replacement of this version'}
                  onclick={() =>
                    void toolsState.run(
                      tool.id,
                      () => toolsApi.repair(item.package.id),
                      'Repairing...',
                    )}>Repair</Button
                >{/if}
              {#if !retained}<Button
                  variant="ghost"
                  size="sm"
                  disabled={disabled || inUse}
                  title={inUse
                    ? 'This version is still in use.'
                    : 'Remove these downloaded files'}
                  onclick={() =>
                    void toolsState.run(
                      tool.id,
                      () => toolsApi.remove(item.package.id),
                      'Removing...',
                    )}>Remove</Button
                >{/if}
            </div>
          </div>
        {/each}
      </div>
    </details>
  {/if}
</div>
