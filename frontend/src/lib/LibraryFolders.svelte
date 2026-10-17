<script lang="ts">
  import { RefreshCw } from '@lucide/svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import { cardBlockClass, cardTableClass } from './ui/styles';
  import { loadRoots, scanRoot, type LibraryRoot } from './title/catalog';

  let {
    timezone,
    timeFormat,
    revision = 0,
    scans = {},
  } = $props<{
    timezone: string;
    timeFormat: '12h' | '24h';
    revision?: number;
    scans?: Record<string, { completed: number; total: number }>;
  }>();
  let roots = $state<LibraryRoot[] | null>(null),
    error = $state(''),
    // The folder's last scan when its scan was queued.
    queued = $state<Record<string, number | null>>({});
  const kinds: Record<string, string> = {
    movies: 'Movies',
    shows: 'Shows',
    music: 'Music',
  };

  async function load() {
    try {
      roots = await loadRoots();
      error = '';
    } catch (caught) {
      error = String(caught);
    }
  }
  $effect(() => {
    void revision;
    void load();
  });

  const waiting = (root: LibraryRoot) =>
    root.id in queued && queued[root.id] === root.last_scan;
  async function scan(root: LibraryRoot) {
    queued[root.id] = root.last_scan;
    try {
      await scanRoot(root.id);
    } catch (caught) {
      error = String(caught);
      delete queued[root.id];
    }
  }
  function scanState(root: LibraryRoot) {
    const progress = scans[root.id];
    if (progress)
      return `Scanning ${progress.completed} of ${progress.total} files`;
    if (waiting(root)) return 'Scan queued';
    if (root.scan_error) return root.scan_error;
    return root.last_scan
      ? `Scanned ${new Date(root.last_scan * 1000).toLocaleString(undefined, { timeZone: timezone, hour12: timeFormat === '12h' })}`
      : 'Waiting for first scan';
  }
</script>

<SettingsCard
  id="server-library"
  title="Library folders"
  description="Folders Thelxinoe scans for movies, shows and music."
>
  {#if error}<p class="m-0 {cardBlockClass} text-xs text-danger" role="alert">
      {error}
    </p>{/if}
  {#if !roots}<div class={cardBlockClass}>
      <ContentSkeleton label="Loading library folders" variant="settings" />
    </div>
  {:else if roots.length}
    <table class={cardTableClass}>
      <thead
        ><tr
          ><th class="text-left">Folder</th><th class="text-left">Scan</th><th
            ><span class="sr-only">Actions</span></th
          ></tr
        ></thead
      >
      <tbody>
        {#each roots as root (root.id)}<tr class="border-b border-line">
            <td
              ><strong class="block font-medium">{root.name}</strong><small
                class="mt-1 block text-[11px] wrap-anywhere text-muted"
                >{kinds[root.kind] ?? root.kind} · {root.path}</small
              ></td
            >
            <td class={root.scan_error ? 'text-danger' : 'text-muted'}
              >{scanState(root)}{#if root.watching === false}<small
                  class="mt-1 block text-[11px] text-muted"
                  >Live updates are unavailable for this folder; changes appear
                  within 15 minutes.</small
                >{/if}</td
            >
            <td class="text-right"
              ><Button
                variant="secondary"
                size="sm"
                disabled={!!scans[root.id] || waiting(root)}
                aria-label={`Scan ${root.name}`}
                onclick={() => void scan(root)}
                ><RefreshCw size={13} />Scan</Button
              ></td
            >
          </tr>{/each}
      </tbody>
    </table>
  {:else}<p class="m-0 {cardBlockClass} text-xs text-muted">
      No library folders yet.
    </p>{/if}
</SettingsCard>
