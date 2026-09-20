<script lang="ts">
  import type { Snippet } from 'svelte';
  import { ChevronDown, RefreshCw } from '@lucide/svelte';
  import {
    isPlugin,
    toolNames,
    type ToolId,
    type ToolView,
    type ToolsSnapshot,
  } from '../../tools-api';
  import {
    availableToolUpdate,
    summarizeTool,
    toolOperationActive,
  } from '../../tools-presentation';
  import type { ToolsState } from '../../tools-state';
  import Button from '../ui/Button.svelte';
  import ToolActionButton from './ToolActionButton.svelte';

  let {
    snapshot,
    expanded,
    pending,
    errors,
    checking,
    onToggle,
    onInstall,
    onCheck,
    onUpdates,
    detail,
  }: {
    snapshot: ToolsSnapshot;
    expanded: ToolId | null;
    pending: ToolsState['pending'];
    errors: ToolsState['errors'];
    checking: ToolId[];
    onToggle: (id: ToolId) => void;
    onInstall: (tool: ToolId, packageId: string) => void;
    onCheck: () => void;
    onUpdates: () => void;
    detail: Snippet<[ToolView]>;
  } = $props();

  function toggleRow(event: MouseEvent, id: ToolId) {
    const target = event.target;
    if (
      target instanceof Element &&
      target.closest('button, a, input, select, textarea, [role="button"]')
    )
      return;
    onToggle(id);
  }
</script>

<div class="@container min-w-0 overflow-x-auto">
  <table class="w-full min-w-md table-fixed border-collapse text-left text-xs">
    <caption class="sr-only"
      >Tools and optional MPV plugins. Expand a row to change its settings.</caption
    >
    <thead
      class="border-b border-(--line) bg-(--surface-soft) text-[0.62rem] text-(--muted)"
    >
      <tr>
        <th scope="col" class="w-1/4 px-3 py-2 font-medium">Tool</th>
        <th
          scope="col"
          class="hidden w-1/5 px-3 py-2 font-medium @min-[46rem]:table-cell"
          >Used copy</th
        >
        <th scope="col" class="w-1/6 px-3 py-2 font-medium">Version</th>
        <th scope="col" class="w-1/6 px-3 py-2 font-medium">
          <div class="flex h-8 items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              class="size-7 shrink-0"
              title="Recheck local tools"
              aria-label="Recheck local tools"
              disabled={checking.length > 0}
              onclick={onCheck}
            >
              <RefreshCw
                class={`size-3.5 ${checking.length ? 'animate-spin motion-reduce:animate-none' : ''}`}
              />
            </Button>
            Status
          </div>
        </th>
        <th scope="col" class="px-3 py-2 font-medium">
          <div class="flex h-8 items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              class="size-7 shrink-0"
              title="Check for updates"
              aria-label="Check for updates"
              disabled={Boolean(pending.updates)}
              onclick={onUpdates}
            >
              <RefreshCw
                class={`size-3.5 ${pending.updates ? 'animate-spin motion-reduce:animate-none' : ''}`}
              />
            </Button>
            Updates
          </div>
        </th>
      </tr>
    </thead>
    {#each [false, true] as plugins (plugins)}
      <tbody>
        <tr class="border-y border-(--line) bg-(--surface-soft)">
          <th
            colspan="5"
            scope="colgroup"
            class="px-3 py-2 text-[0.62rem] font-medium text-(--muted)"
            >{plugins ? 'Optional MPV plugins' : 'MPV player'}</th
          >
        </tr>
        {#each snapshot.tools.filter((tool) => isPlugin(tool.id) === plugins) as tool (tool.id)}
          {@const summary = summarizeTool(tool, snapshot.mpv.source)}
          {@const update = availableToolUpdate(tool)}
          {@const operation =
            snapshot.operation.tool === tool.id ? snapshot.operation : null}
          {@const installing =
            toolOperationActive(operation) ||
            ['Installing...', 'Activating...', 'Repairing...'].includes(
              pending[tool.id] ?? '',
            )}
          {@const isChecking = checking.includes(tool.id)}
          <tr
            class={`cursor-default border-b border-(--line) ${expanded === tool.id ? 'bg-(--accent-soft)' : 'hover:bg-(--surface-soft)'}`}
            onclick={(event) => toggleRow(event, tool.id)}
          >
            <th scope="row" class="px-3 py-3 font-normal">
              <button
                id={'tool-trigger-' + tool.id}
                type="button"
                class="flex w-full items-start gap-2 text-left"
                aria-expanded={expanded === tool.id}
                aria-controls={'tool-panel-' + tool.id}
                onclick={() => onToggle(tool.id)}
              >
                <ChevronDown
                  class={`mt-0.5 size-3.5 shrink-0 transition-transform ${expanded === tool.id ? 'rotate-180' : ''}`}
                />
                <span class="min-w-0">
                  <span class="block font-semibold">{toolNames[tool.id]}</span>
                  {#if errors[tool.id] || (tool.id === 'mpv' && errors.configuration)}<span
                      class="mt-1 block text-[0.62rem] text-(--danger)"
                      >Action failed. Expand for details.</span
                    >{/if}
                  <span
                    class="mt-1 block text-[0.62rem] text-(--muted) @min-[46rem]:hidden"
                    >{summary.source}</span
                  >
                </span>
              </button>
            </th>
            <td class="hidden px-3 py-3 text-(--muted) @min-[46rem]:table-cell"
              >{summary.source}</td
            >
            <td class="px-3 py-3"
              ><span
                class="block truncate font-mono text-[0.65rem]"
                title={summary.version}>{summary.version}</span
              >{#if update}<span
                  class="mt-1 block truncate text-[0.62rem] text-(--accent)"
                  title={`Available: ${update.version}`}
                  >New: {update.version}</span
                >{/if}</td
            >
            <td class="px-3 py-3">
              <span
                class={`inline-flex items-center gap-1.5 rounded-sm px-2 py-1 text-[0.62rem] ${isChecking || summary.tone === 'muted' ? 'bg-(--surface-soft) text-(--muted)' : summary.tone === 'warning' ? 'bg-[color-mix(in_srgb,var(--warning)_10%,transparent)] text-(--warning)' : 'bg-[color-mix(in_srgb,var(--success)_10%,transparent)] text-(--success)'}`}
              >
                {#if isChecking}<RefreshCw
                    class="size-3 shrink-0 animate-spin motion-reduce:animate-none"
                  />{/if}{isChecking ? 'Checking...' : summary.status}
              </span>
            </td>
            <td class="px-3 py-3 text-(--muted)">
              {#if update || installing}
                <ToolActionButton
                  label={update ? 'Update' : 'Install'}
                  pending={installing}
                  {operation}
                  disabled={isChecking || !update}
                  onclick={() => {
                    if (update) onInstall(tool.id, update.id);
                  }}
                />
              {:else}<span class="text-[0.62rem]">{summary.updates}</span>{/if}
            </td>
          </tr>
          {#if expanded === tool.id}
            <tr
              ><td colspan="5" class="border-b border-(--line) bg-(--surface)">
                <section
                  id={'tool-panel-' + tool.id}
                  aria-labelledby={'tool-trigger-' + tool.id}
                  class="grid w-full max-w-4xl gap-4 px-3 py-4"
                >
                  {@render detail(tool)}
                </section>
              </td></tr
            >
          {/if}
        {/each}
      </tbody>
    {/each}
  </table>
</div>
