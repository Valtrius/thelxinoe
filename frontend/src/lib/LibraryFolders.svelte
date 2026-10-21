<script lang="ts">
  import { FolderPlus, Pencil, RefreshCw, Trash2 } from '@lucide/svelte';
  import { twMerge } from 'tailwind-merge';
  import SettingsCard from './ui/SettingsCard.svelte';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import FormField from './ui/FormField.svelte';
  import ConfirmDialog from './providers/components/ui/ConfirmDialog.svelte';
  import { reveal } from './ui/reveal';
  import { withVerification } from './authentication';
  import {
    cardBlockClass,
    cardTableClass,
    formControlClass,
  } from './ui/styles';
  import {
    addRoot,
    loadLibraryFolders,
    removeRoot,
    renameRoot,
    scanRoot,
    type LibraryRoot,
  } from './title/catalog';

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
    mount = $state(''),
    error = $state(''),
    // The folder's last scan when its scan was queued.
    queued = $state<Record<string, number | null>>({}),
    busy = $state(false),
    adding = $state(false),
    draft = $state({ name: '', kind: 'movies', path: '' }),
    renaming = $state<string | null>(null),
    name = $state(''),
    removing = $state<LibraryRoot | null>(null),
    removalError = $state('');
  const kinds: Record<string, string> = {
    movies: 'Movies',
    shows: 'Shows',
    music: 'Music',
  };

  async function load() {
    try {
      const loaded = await loadLibraryFolders();
      roots = loaded.items;
      mount = loaded.media_mount;
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
  async function change(action: () => Promise<unknown>) {
    busy = true;
    error = '';
    try {
      await action();
      await load();
      return true;
    } catch (caught) {
      error = String(caught);
      return false;
    } finally {
      busy = false;
    }
  }
  async function add() {
    const root = { ...draft, name: draft.name.trim(), path: draft.path.trim() };
    if (await change(() => addRoot(root))) {
      adding = false;
      draft = { name: '', kind: 'movies', path: '' };
    }
  }
  async function rename(root: LibraryRoot) {
    if (await change(() => renameRoot(root.id, name.trim()))) renaming = null;
  }
  async function remove(root: LibraryRoot) {
    busy = true;
    removalError = '';
    try {
      const removed = await withVerification(async () => {
        await removeRoot(root.id);
      });
      if (!removed) return;
      removing = null;
      await load();
    } catch (caught) {
      removalError = String(caught);
    } finally {
      busy = false;
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
  {#snippet actions()}
    {#if !adding}<Button
        variant="secondary"
        size="sm"
        onclick={() => (adding = true)}
        ><FolderPlus size={13} />Add folder</Button
      >{/if}
  {/snippet}
  {#if error}<p class="m-0 {cardBlockClass} text-xs text-danger" role="alert">
      {error}
    </p>{/if}
  {#if adding}
    <form
      transition:reveal
      aria-label="Add a library folder"
      class="{cardBlockClass} grid gap-3 bg-surface-soft"
      onsubmit={(event) => {
        event.preventDefault();
        void add();
      }}
    >
      <div
        class="grid max-w-[36rem] grid-cols-2 gap-3 compact:grid-cols-1 [&_label]:m-0"
      >
        <FormField
          >Name<input
            class={formControlClass}
            maxlength="100"
            required
            bind:value={draft.name}
          /></FormField
        >
        <FormField
          >Media type<select class={formControlClass} bind:value={draft.kind}
            >{#each Object.entries(kinds) as [value, label] (value)}<option
                {value}>{label}</option
              >{/each}</select
          ></FormField
        >
        <FormField class="col-span-full settings-control:max-w-none"
          >Folder<input
            class={formControlClass}
            placeholder={mount ? `${mount}/movies` : ''}
            required
            autocomplete="off"
            bind:value={draft.path}
          /></FormField
        >
      </div>
      <p class="m-0 text-xs text-muted">
        The folder must be inside {mount || 'the media folder'} and can't overlap
        another library folder. Its location and media type can't be changed later;
        a folder added again is scanned as new.
      </p>
      <div class="flex flex-wrap gap-2">
        <Button
          size="form"
          type="submit"
          loading={busy}
          disabled={!draft.name.trim() || !draft.path.trim()}>Add folder</Button
        >
        <Button variant="ghost" size="form" onclick={() => (adding = false)}
          >Cancel</Button
        >
      </div>
    </form>
  {/if}
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
              >{#if renaming === root.id}<form
                  class="flex flex-wrap items-center gap-2"
                  onsubmit={(event) => {
                    event.preventDefault();
                    void rename(root);
                  }}
                >
                  <input
                    class={twMerge(formControlClass, 'w-48 py-1.5')}
                    aria-label={`New name for ${root.name}`}
                    maxlength="100"
                    required
                    bind:value={name}
                    {@attach (element) => element.focus()}
                  />
                  <Button
                    size="sm"
                    type="submit"
                    disabled={busy || !name.trim()}>Save</Button
                  >
                  <Button
                    variant="ghost"
                    size="sm"
                    onclick={() => (renaming = null)}>Cancel</Button
                  >
                </form>{:else}<strong class="block font-medium"
                  >{root.name}</strong
                >{/if}<small
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
            <td class="text-right whitespace-nowrap"
              ><div class="inline-flex gap-1">
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={!!scans[root.id] || waiting(root)}
                  aria-label={`Scan ${root.name}`}
                  onclick={() => void scan(root)}
                  ><RefreshCw size={13} />Scan</Button
                ><Button
                  variant="ghost"
                  size="sm"
                  class="w-8 px-0"
                  disabled={busy}
                  title={`Rename ${root.name}`}
                  aria-label={`Rename ${root.name}`}
                  onclick={() => {
                    name = root.name;
                    renaming = root.id;
                  }}><Pencil size={13} aria-hidden="true" /></Button
                ><Button
                  variant="ghost"
                  size="sm"
                  class="w-8 px-0"
                  disabled={busy}
                  title={`Remove ${root.name}`}
                  aria-label={`Remove ${root.name}`}
                  onclick={() => {
                    removalError = '';
                    removing = root;
                  }}><Trash2 size={13} aria-hidden="true" /></Button
                >
              </div></td
            >
          </tr>{/each}
      </tbody>
    </table>
  {:else}<p class="m-0 {cardBlockClass} text-xs text-muted">
      No library folders yet.
    </p>{/if}
</SettingsCard>
<ConfirmDialog
  open={removing !== null}
  title={`Remove ${removing?.name ?? 'library folder'}?`}
  message="Its titles leave the library, along with everyone's watched marks, favorites and progress for them. Files on disk aren't touched and watch history is kept. Adding the folder again scans it as new."
  confirmLabel="Remove folder"
  eyebrow="REMOVE / LIBRARY FOLDER"
  danger
  {busy}
  error={removalError}
  onCancel={() => (removing = null)}
  onConfirm={() => {
    if (removing) void remove(removing);
  }}
/>
