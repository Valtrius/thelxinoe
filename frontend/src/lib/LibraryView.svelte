<script lang="ts">
  import { Folder, RefreshCw, ArrowLeft } from '@lucide/svelte';
  import { api } from './api';
  import MediaGrid from './ui/MediaGrid.svelte';
  import LibraryCard from './ui/LibraryCard.svelte';
  import MetadataEditor from './MetadataEditor.svelte';
  import EpisodeMapping from './EpisodeMapping.svelte';
  import MediaActions from './MediaActions.svelte';
  import MediaOperations from './MediaOperations.svelte';
  import SegmentEditor from './SegmentEditor.svelte';
  import Requests from './Requests.svelte';
  import { untrack, onDestroy } from 'svelte';
  import type { MediaChoice } from './playback';
  let {
    domain,
    admin,
    revision = 0,
    scans = {},
    play,
    userId,
    focusId,
  } = $props<{
    domain: string;
    admin: boolean;
    revision?: number;
    scans?: Record<string, { completed: number; total: number }>;
    play: (choice: MediaChoice) => void;
    userId: string;
    focusId?: string;
  }>();
  type Item = {
    id: string;
    kind: string;
    title: string;
    year: number | null;
    available: boolean;
    metadata: Record<string, unknown>;
    artwork_url?: string;
    overview?: string;
    overrides?: { title?: string; overview?: string; year?: number };
  };
  type Root = {
    id: string;
    name: string;
    kind: string;
    path: string;
    last_scan: number | null;
    scan_error: string | null;
  };
  let items = $state<Item[]>([]),
    roots = $state<Root[]>([]),
    error = $state(''),
    busy = $state(false),
    showAdd = $state(false),
    name = $state(''),
    path = $state(''),
    mount = $state('');
  let collections = $state<{ id: number; name: string; count: number }[]>([]),
    collection = $state('');
  let breadcrumbs = $state<{ id: string; title: string }[]>([]),
    selected = $state<Item | null>(null),
    details = $state<{
      files: {
        id: string;
        edition: string;
        present: boolean;
        probe: { format?: { duration?: string } };
      }[];
      local_trailers?: { file_id: string }[];
    } | null>(null);
  let generation = 0;
  let search = $state('');
  let searchTimer: ReturnType<typeof setTimeout>;
  onDestroy(() => {
    clearTimeout(searchTimer);
    generation++;
  });
  function searchChanged() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void load(), 250);
  }
  let acquisition = $state(false);
  async function load() {
    const request = ++generation;
    busy = true;
    error = '';
    try {
      const parent = breadcrumbs.at(-1)?.id;
      const query = parent
        ? `parent=${parent}`
        : `kind=${domain === 'Movies' ? 'movie' : domain === 'Shows' ? 'show' : 'artist'}`;
      const result = await api<{ items: Item[] }>(
        `/catalog?${query}&q=${encodeURIComponent(search)}${collection ? `&collection=${collection}` : ''}`,
      );
      if (request !== generation) return;
      items = result.items;
      if (selected) {
        const detail = await api<Item & NonNullable<typeof details>>(
          `/catalog/${selected.id}`,
        );
        if (request !== generation) return;
        selected = detail;
        details = detail;
      }
      if (domain === 'Movies') {
        const result = await api<{ items: typeof collections }>(
          '/catalog/collections',
        );
        if (request !== generation) return;
        collections = result.items;
      }
      if (admin) {
        const result = await api<{ items: Root[]; media_mount: string }>(
          '/catalog/roots',
        );
        if (request !== generation) return;
        roots = result.items.filter((r) => r.kind === domain.toLowerCase());
        mount = result.media_mount;
      }
    } catch (e) {
      if (request === generation) error = String(e);
    } finally {
      if (request === generation) busy = false;
    }
  }
  $effect(() => {
    if (domain)
      untrack(() => {
        breadcrumbs = [];
        selected = null;
        details = null;
        collection = '';
        void load();
      });
  });
  $effect(() => {
    if (revision > 0) untrack(() => void load());
  });
  $effect(() => {
    const id = focusId;
    if (id)
      untrack(
        () =>
          void api<Item>(`/catalog/${id}`)
            .then((item) => open(item))
            .catch((e) => (error = String(e))),
      );
  });
  async function open(item: Item) {
    if (['movie', 'episode', 'track'].includes(item.kind)) {
      await select(item);
    } else {
      breadcrumbs = [...breadcrumbs, { id: item.id, title: item.title }];
      await load();
    }
  }
  async function select(item: Item) {
    selected = item;
    details = null;
    try {
      const detail = await api<Item & NonNullable<typeof details>>(
        `/catalog/${item.id}`,
      );
      if (selected?.id === item.id) {
        selected = detail;
        details = detail;
      }
    } catch (e) {
      error = String(e);
    }
  }
  async function add() {
    busy = true;
    error = '';
    try {
      await api('/catalog/roots', 'POST', {
        name,
        path,
        kind: domain.toLowerCase(),
      });
      name = '';
      path = '';
      showAdd = false;
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="section-heading">
  <div>
    {#if breadcrumbs.length}<button
        class="secondary"
        onclick={() => {
          breadcrumbs = breadcrumbs.slice(0, -1);
          void load();
        }}><ArrowLeft size={15} />{breadcrumbs.at(-1)?.title}</button
      >{:else}<p class="muted">Your collection, in one place.</p>{/if}
  </div>
  <button class="secondary" onclick={() => (acquisition = !acquisition)}
    >{acquisition ? 'Hide requests' : 'Search and request'}</button
  >
  {#if admin}<button class="secondary" onclick={() => (showAdd = !showAdd)}
      ><Folder size={16} /> Add folder</button
    >{/if}
</div>
<div class="view-toolbar">
  <label class="search-field"
    >Search {domain.toLowerCase()}<input
      bind:value={search}
      oninput={searchChanged}
      placeholder={`Search your ${domain.toLowerCase()}`}
    /></label
  ><span class="muted">{items.length} items · Ctrl + scroll to zoom</span>
</div>
{#if acquisition}{#key domain}<Requests
      user={{ id: userId, role: admin ? 'admin' : 'user' }}
      {domain}
    />{/key}{/if}
{#if error}<p class="error" role="alert">{error}</p>{/if}
{#if domain === 'Movies' && collections.length}
  <label class="collection-filter"
    >Collection<select bind:value={collection} onchange={() => void load()}
      ><option value="">All movies</option
      >{#each collections as group (group.id)}<option value={String(group.id)}
          >{group.name} ({group.count})</option
        >{/each}</select
    ></label
  >
{/if}
{#if showAdd}<form
    class="panel inline-form"
    onsubmit={(e) => {
      e.preventDefault();
      void add();
    }}
  >
    <label
      >Library name<input
        bind:value={name}
        required
        placeholder={`My ${domain.toLowerCase()}`}
      /></label
    ><label
      >Folder inside {mount}<input
        bind:value={path}
        required
        placeholder={`${mount}/${domain.toLowerCase()}`}
      /></label
    ><button class="primary" disabled={busy}>Add and scan</button>
  </form>{/if}
{#if admin && roots.length}<details class="library-roots">
    <summary>Library folders · {roots.length}</summary>
    <div>
      {#each roots as root (root.id)}<div class="row">
          <div>
            <strong>{root.name}</strong><small
              >{scans[root.id]
                ? `Scanning ${scans[root.id].completed} of ${scans[root.id].total} files`
                : (root.scan_error ??
                  (root.last_scan
                    ? `Scanned ${new Date(root.last_scan * 1000).toLocaleString()}`
                    : 'Waiting for first scan'))}</small
            >
          </div>
          <button
            class="secondary"
            disabled={busy}
            onclick={async () => {
              try {
                await api(`/catalog/roots/${root.id}/scan`, 'POST');
              } catch (e) {
                error = String(e);
              }
            }}><RefreshCw size={14} /> Scan</button
          >
        </div>{/each}
    </div>
  </details>{/if}
{#if selected}<section class="panel">
    <div class="section-heading">
      <h2>{selected.title}</h2>
      <button class="secondary" onclick={() => (selected = null)}>Close</button>
    </div>
    {#if selected.overview}<p class="muted">{selected.overview}</p>{/if}
    <MediaActions id={selected.id} kind={selected.kind} {userId} />
    {#if admin}<MediaOperations
        id={selected.id}
        changed={() => void load()}
      />{/if}
    {#if details?.local_trailers?.length}<p class="muted">
        {details.local_trailers.length} local trailer(s) indexed.
      </p>{/if}
    {#each details?.files ?? [] as file (file.id)}<div class="row">
        <span>{file.edition || 'Original edition'}</span><span class="muted"
          >{file.probe.format?.duration
            ? `${Math.round(Number(file.probe.format.duration) / 60)} minutes`
            : 'Duration unknown'}</span
        >
        {#if file.present}<button
            class="primary"
            onclick={() =>
              play({
                id: selected!.id,
                title: selected!.title,
                fileId: file.id,
                kind: selected!.kind,
                queue:
                  selected!.kind === 'track'
                    ? items.filter((i) => i.kind === 'track' && i.available)
                    : undefined,
              })}>Play {file.edition || 'media'}</button
          >{/if}
      </div>
      {#if admin && file.present && ['movie', 'episode'].includes(selected.kind)}<SegmentEditor
          episode={selected.kind === 'episode'}
          mediaId={selected.id}
          fileId={file.id}
          duration={Number(file.probe.format?.duration ?? 0)}
        />{/if}
    {/each}
    {#if admin && details}<MetadataEditor
        id={selected.id}
        kind={selected.kind}
        title={selected.title}
        overrides={selected.overrides}
        changed={() => void load()}
      />{#if selected.kind === 'episode'}<EpisodeMapping
          id={selected.id}
          changed={() => void load()}
        />{/if}{/if}
  </section>{/if}
{#if items.length}<MediaGrid
    revision={items.map((i) => i.id).join(',')}
    label={domain}
  >
    {#each items as item (item.id)}<LibraryCard
        {item}
        open={() => void open(item)}
        details={() => void select(item)}
        play={['movie', 'episode', 'track'].includes(item.kind) &&
        item.available
          ? () =>
              play({
                id: item.id,
                title: item.title,
                kind: item.kind,
                queue:
                  item.kind === 'track'
                    ? items.filter((i) => i.kind === 'track' && i.available)
                    : undefined,
              })
          : undefined}
      />{/each}
  </MediaGrid>{:else}<section class="empty">
    <Folder size={42} />
    <h2>{busy ? 'Loading your library…' : 'Your collection starts here'}</h2>
    <p>
      {admin
        ? 'Add a folder from your media mount. Thelxinoe will discover its files and keep the catalog up to date.'
        : 'Your administrator can add media folders to this library.'}
    </p>
  </section>{/if}

<style>
  .library-roots {
    margin-bottom: 20px;
    border-bottom: 1px solid var(--line);
  }
  .collection-filter {
    max-width: 320px;
  }
</style>
