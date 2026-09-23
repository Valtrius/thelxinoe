<script lang="ts">
  import ProgressBar from '../ui/ProgressBar.svelte';
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
    class="downloads-scroll max-h-136 overflow-auto [scrollbar-width:thin] [scrollbar-color:var(--line-strong)_transparent]"
    tabindex="0"
    role="region"
    aria-label="Current and recent downloads"
  >
    <table
      class="w-full min-w-0 table-fixed border-separate border-spacing-0 text-[11px] [&_th]:sticky [&_th]:top-0 [&_th]:z-1 [&_th]:h-8.5 [&_th]:bg-surface-strong [&_th]:text-[9px] [&_th]:font-[650] [&_th]:tracking-[0.06em] [&_th]:text-muted [&_th]:uppercase [&_th]:border-b [&_th]:border-line [&_th]:px-1.5 [&_th]:py-2.5 [&_th]:text-left [&_th]:align-middle [&_td]:border-b [&_td]:border-line [&_td]:px-1.5 [&_td]:py-2.5 [&_td]:text-left [&_td]:align-middle compact:text-[9px] compact:[&_th]:px-0.75 compact:[&_th]:py-2.25 compact:[&_td]:px-0.75 compact:[&_td]:py-2.25 tight:[&_td]:border-0 tight:[&_td]:px-0.5 tight:[&_td]:py-0.75"
    >
      <thead class="tight:sr-only"
        ><tr
          ><th class="state-col w-8 compact:w-5.5" scope="col"
            ><span class="sr-only">Status</span></th
          ><th class="name-col w-[29%] compact:w-1/4" scope="col">Download</th
          ><th class="progress-col w-auto" scope="col">Progress</th><th
            class="actions-col w-17 compact:w-14.5"
            scope="col">Actions</th
          ></tr
        ></thead
      >
      <tbody class="tight:block">
        {#each rows as row (`${row.history ? 'history' : 'queue'}:${row.id}`)}
          {@const state = downloadState(row, paused)}
          {@const progress = downloadProgress(row)}
          {@const StateIcon = icons[state.icon as keyof typeof icons]}
          <tr
            class="h-12.5 tight:grid tight:h-auto tight:min-h-16.25 tight:grid-cols-[20px_minmax(0,1fr)_54px] tight:border-b tight:border-line tight:py-1.5 tight:[&>td:nth-child(3)]:col-start-2 tight:[&>td:nth-child(3)]:self-center tight:[&>td:last-child]:col-start-3 tight:[&>td:last-child]:self-center"
          >
            <td
              class="download-state text-muted data-[tone=accent]:text-accent data-[tone=ok]:text-success data-[tone=warn]:text-warning data-[tone=bad]:text-danger tight:row-span-2 tight:self-center"
              data-tone={state.tone}
            >
              <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard focus reveals the same status detail as hover.) -->
              <span
                class="state-symbol group/status inline-flex cursor-help"
                tabindex="0"
                role="img"
                aria-label={state.label}
                title={state.label}
              >
                <StateIcon
                  size={15}
                  class={state.icon === 'processing'
                    ? 'animate-spin [animation-duration:1.4s] motion-reduce:animate-none'
                    : ''}
                  aria-hidden="true"
                />
                <span
                  class="status-tooltip absolute z-2 mt-5 hidden max-w-57.5 border border-line-strong bg-surface-strong px-2 py-1.25 text-[11px] text-foreground group-hover/status:block group-focus/status:block"
                  aria-hidden="true">{state.label}</span
                >
              </span>
            </td>
            <td
              class="download-name font-[550] wrap-anywhere tight:col-start-2 tight:col-end-4 tight:text-[10px]"
              title={row.title}>{row.title}</td
            >
            <td>
              <div
                class="progress-values grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] gap-0.75 text-[10px] whitespace-nowrap tabular-nums [&_strong]:font-[650] [&>span:last-child]:text-right compact:gap-0.5 compact:text-[8px]"
              >
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
              <ProgressBar
                value={progress.percent}
                label={`${row.title} download progress`}
                class={`download-progress mt-1.5 ${state.tone === 'ok' ? 'text-success' : state.tone === 'bad' ? 'text-danger' : 'text-accent'}`}
                barClass="transition-[width] duration-350 ease-[ease] motion-reduce:transition-none"
              />
            </td>
            <td>
              {#if !row.history}
                <div
                  class="download-actions flex justify-end gap-0.5 [&_button]:h-6.5 [&_button]:w-6.5 compact:[&_button]:w-5.75"
                >
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
