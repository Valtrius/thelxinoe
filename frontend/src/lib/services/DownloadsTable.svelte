<script lang="ts">
  import {
    ArrowDownToLine,
    Pause,
    Play,
    Trash2,
    Clock3,
    Check,
    CircleX,
    TriangleAlert,
    CircleHelp,
    LoaderCircle,
  } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import {
    downloadRows,
    downloadState,
    downloadProgress,
    formatDownloadSize,
    type SupportDownload,
  } from './downloads';

  let {
    queue,
    history,
    paused = false,
    busy,
    onaction,
  }: {
    queue: SupportDownload[];
    history: SupportDownload[];
    paused?: boolean;
    busy: boolean;
    onaction: (action: string, id: number) => void;
  } = $props();
  const rows = $derived(downloadRows(queue, history));
  const icons = {
    download: ArrowDownToLine,
    pause: Pause,
    queue: Clock3,
    complete: Check,
    error: CircleX,
    warning: TriangleAlert,
    removed: Trash2,
    processing: LoaderCircle,
    unknown: CircleHelp,
  };
</script>

{#if rows.length === 0}
  <p class="text-xs text-muted">No current or recent downloads.</p>
{:else}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scroll region must be keyboard scrollable.) -->
  <div
    class="downloads-scroll"
    tabindex="0"
    role="region"
    aria-label="Current and recent downloads"
  >
    <table>
      <thead
        ><tr
          ><th class="state-col" scope="col"
            ><span class="sr-only">Status</span></th
          ><th class="name-col" scope="col">Download</th><th
            class="progress-col"
            scope="col">Progress</th
          ><th class="actions-col" scope="col">Actions</th></tr
        ></thead
      >
      <tbody>
        {#each rows as row (`${row.history ? 'history' : 'queue'}:${row.id}`)}
          {@const state = downloadState(row, paused)}
          {@const progress = downloadProgress(row)}
          {@const StateIcon = icons[state.icon as keyof typeof icons]}
          <tr>
            <td class="download-state" data-tone={state.tone}>
              <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard focus reveals the same status detail as hover.) -->
              <span
                class="state-symbol"
                tabindex="0"
                role="img"
                aria-label={state.label}
                title={state.label}
              >
                <StateIcon
                  size={15}
                  class={state.icon === 'processing' ? 'processing' : ''}
                  aria-hidden="true"
                />
                <span class="status-tooltip" aria-hidden="true"
                  >{state.label}</span
                >
              </span>
            </td>
            <td class="download-name" title={row.title}>{row.title}</td>
            <td>
              <div class="progress-values">
                <span title="Downloaded"
                  ><span class="sr-only">Downloaded </span>{formatDownloadSize(
                    progress.downloaded,
                  )}</span
                >
                <strong
                  >{progress.percent === null
                    ? '—'
                    : `${progress.percent}%`}</strong
                >
                <span title="Remaining"
                  ><span class="sr-only">Remaining </span>{formatDownloadSize(
                    progress.remaining,
                  )}</span
                >
              </div>
              <div
                class="download-progress"
                data-tone={state.tone}
                role="progressbar"
                aria-label={`${row.title} download progress`}
                aria-valuenow={progress.percent ?? undefined}
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuetext={progress.percent === null
                  ? 'Progress unavailable'
                  : `${progress.percent}%`}
              >
                <span style:width={`${progress.percent ?? 0}%`}></span>
              </div>
            </td>
            <td>
              {#if !row.history}
                <div class="download-actions">
                  {#if ['QUEUED', 'PAUSED', 'DOWNLOADING', 'FETCHING'].includes(row.status)}
                    <Button
                      variant="secondary"
                      size="compact-icon"
                      disabled={busy}
                      title={`${row.status === 'PAUSED' ? 'Resume' : 'Pause'} ${row.title}`}
                      aria-label={`${row.status === 'PAUSED' ? 'Resume' : 'Pause'} ${row.title}`}
                      onclick={() =>
                        onaction(
                          row.status === 'PAUSED' ? 'resume' : 'pause',
                          row.id,
                        )}
                    >
                      {#if row.status === 'PAUSED'}<Play
                          size={14}
                          aria-hidden="true"
                        />{:else}<Pause size={14} aria-hidden="true" />{/if}
                    </Button>
                  {/if}
                  <Button
                    variant="secondary"
                    size="compact-icon"
                    disabled={busy}
                    title={`Remove ${row.title}`}
                    aria-label={`Remove ${row.title}`}
                    onclick={() => onaction('remove', row.id)}
                    ><Trash2 size={14} aria-hidden="true" /></Button
                  >
                </div>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .downloads-scroll {
    max-height: 34rem;
    overflow: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--line-strong) transparent;
  }
  table {
    width: 100%;
    min-width: 0;
    table-layout: fixed;
    border-collapse: separate;
    border-spacing: 0;
    font-size: 11px;
  }
  th,
  td {
    border-bottom: 1px solid var(--line);
    padding: 10px 6px;
    vertical-align: middle;
    text-align: left;
  }
  th {
    position: sticky;
    top: 0;
    z-index: 1;
    height: 34px;
    background: var(--surface-strong);
    color: var(--muted);
    font-size: 9px;
    font-weight: 650;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  tbody tr {
    height: 50px;
  }
  .state-col {
    width: 32px;
  }
  .name-col {
    width: 29%;
  }
  .progress-col {
    width: auto;
  }
  .actions-col {
    width: 68px;
  }
  .download-name {
    overflow-wrap: anywhere;
    font-weight: 550;
  }
  .state-symbol {
    display: inline-flex;
    cursor: help;
  }
  .status-tooltip {
    display: none;
    position: absolute;
    z-index: 2;
    margin: 20px 0 0;
    max-width: 230px;
    padding: 5px 8px;
    background: var(--surface-strong);
    border: 1px solid var(--line-strong);
    color: var(--foreground);
    font-size: 11px;
  }
  .state-symbol:hover .status-tooltip,
  .state-symbol:focus .status-tooltip {
    display: block;
  }
  .download-state {
    color: var(--muted);
  }
  [data-tone='accent'] {
    color: var(--accent);
  }
  [data-tone='ok'] {
    color: var(--success);
  }
  [data-tone='warn'] {
    color: var(--warning);
  }
  [data-tone='bad'] {
    color: var(--danger);
  }
  .progress-values {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    gap: 3px;
    font-size: 10px;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .progress-values strong {
    font-weight: 650;
  }
  .progress-values > span:last-child {
    text-align: right;
  }
  .download-progress {
    height: 4px;
    margin-top: 6px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .download-progress[data-tone='ok'] {
    color: var(--success);
  }
  .download-progress[data-tone='bad'] {
    color: var(--danger);
  }
  .download-progress > span {
    display: block;
    height: 100%;
    background: currentColor;
    transition: width 0.35s ease;
  }
  .download-actions {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
  }
  .download-actions :global(button) {
    width: 26px;
    height: 26px;
  }
  .state-symbol :global(.processing) {
    animation: turn 1.4s linear infinite;
  }
  @keyframes turn {
    to {
      transform: rotate(360deg);
    }
  }
  @media (max-width: 720px) {
    table {
      min-width: 0;
      font-size: 9px;
    }
    th,
    td {
      padding: 9px 3px;
    }
    .state-col {
      width: 22px;
    }
    .name-col {
      width: 25%;
    }
    .progress-col {
      width: auto;
    }
    .actions-col {
      width: 58px;
    }
    .progress-values {
      font-size: 8px;
      gap: 2px;
    }
    .download-actions :global(button) {
      width: 23px;
      height: 26px;
    }
  }
  @media (max-width: 440px) {
    thead {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip-path: inset(50%);
    }
    tbody {
      display: block;
    }
    tbody tr {
      display: grid;
      grid-template-columns: 20px minmax(0, 1fr) 54px;
      height: auto;
      min-height: 65px;
      border-bottom: 1px solid var(--line);
      padding: 6px 0;
    }
    td {
      border: 0;
      padding: 3px 2px;
    }
    .download-state {
      grid-row: 1/3;
      align-self: center;
    }
    .download-name {
      grid-column: 2/4;
      font-size: 10px;
    }
    td:nth-child(3) {
      grid-column: 2;
      align-self: center;
    }
    td:last-child {
      grid-column: 3;
      align-self: center;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .state-symbol :global(.processing) {
      animation: none;
    }
    .download-progress > span {
      transition: none;
    }
  }
</style>
