<script lang="ts">
  import { onMount } from 'svelte';
  import { Download, Search, TriangleAlert } from '@lucide/svelte';
  import Modal from '../ui/Modal.svelte';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import Switch from '../ui/Switch.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import ExclusiveChoiceGroup from '../ui/ExclusiveChoiceGroup.svelte';
  import { formatBytes, relativeTime } from '../providers/utils';
  import { showToast } from '../providers/toasts';
  import {
    grabRelease,
    loadReleases,
    type Release,
    type ReleaseTarget,
  } from './sonarr';

  let { series, target, title, close } = $props<{
    series: string;
    target: ReleaseTarget;
    /** What is searched for, such as S02E06 · Title. */
    title: string;
    close: () => void;
  }>();
  let releases = $state<Release[] | null>(null),
    error = $state(''),
    query = $state(''),
    resolution = $state('all'),
    rejected = $state(false),
    grabbing = $state<string | null>(null),
    // A rejected release downloads only once the second click confirms it.
    confirming = $state<string | null>(null);

  const resolutionOf = (release: Release) =>
    release.quality?.match(/(\d{3,4})p/)?.[1] ?? null;
  const resolutions = $derived(
    [
      ...new Set(
        (releases ?? [])
          .map(resolutionOf)
          .filter((value): value is string => value !== null),
      ),
    ].sort((a, b) => Number(b) - Number(a)),
  );
  const best = $derived(releases?.find((release) => release.approved));
  const hidden = $derived(
    rejected ? 0 : (releases ?? []).filter((r) => !r.approved).length,
  );
  const shown = $derived(
    (releases ?? []).filter(
      (release) =>
        (rejected || release.approved) &&
        (resolution === 'all' || resolutionOf(release) === resolution) &&
        release.title.toLowerCase().includes(query.trim().toLowerCase()),
    ),
  );

  async function search() {
    releases = null;
    error = '';
    try {
      releases = await loadReleases(series, target);
    } catch (caught) {
      error = String(caught);
    }
  }
  onMount(() => void search());

  async function grab(release: Release) {
    if (!release.approved && confirming !== release.guid) {
      confirming = release.guid;
      return;
    }
    grabbing = release.guid;
    try {
      await grabRelease(series, release);
      showToast({
        key: `release:${series}`,
        tone: 'success',
        title,
        message: 'Sent to the download client.',
      });
      close();
    } catch (caught) {
      error = String(caught);
    } finally {
      grabbing = confirming = null;
    }
  }
  const published = (seconds: number | null) =>
    seconds ? relativeTime(new Date(seconds * 1000).toISOString()) : '';
</script>

<Modal title={`Interactive search · ${title}`} size="wide" onClose={close}>
  <div class="mb-3 flex flex-wrap items-center gap-2">
    <label
      class="flex h-8 min-w-48 flex-1 items-center gap-2 border border-line bg-surface-soft px-2.5 text-xs text-muted focus-within:border-line-strong"
    >
      <Search size={13} class="shrink-0" />
      <input
        class="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted"
        placeholder="Filter releases"
        aria-label="Filter releases"
        bind:value={query}
      />
    </label>
    {#if resolutions.length > 1}<ExclusiveChoiceGroup
        ariaLabel="Resolution"
        value={resolution}
        choices={[
          { value: 'all', label: 'All' },
          ...resolutions.map((value) => ({ value, label: `${value}p` })),
        ]}
        onChange={(value) => (resolution = value)}
      />{/if}
    <Switch size="sm" bind:checked={rejected}>Show rejected</Switch>
  </div>
  {#if error}<Notice variant="error" role="alert"
      >{error}<Button variant="ghost" size="sm" onclick={() => void search()}
        >Search again</Button
      ></Notice
    >{/if}
  {#if releases === null && !error}<ContentSkeleton
      label="Searching indexers"
    />{:else if releases && !shown.length}<p class="py-6 text-sm text-muted">
      {releases.length ? 'No release matches.' : 'Sonarr found no releases.'}
    </p>{:else if releases}
    <table class="w-full table-fixed border-collapse text-[11px]">
      <colgroup>
        <col />
        <col class="w-26" />
        <col class="w-16" />
        <col class="w-13" />
        <col class="w-14" />
        <col class="w-20" />
      </colgroup>
      <thead>
        <tr
          class="border-b border-line text-left text-[9px] font-semibold tracking-[0.12em] text-muted uppercase"
        >
          <th class="py-2 pr-2 font-semibold">Release</th>
          <th class="px-1.5 py-2 font-semibold">Quality</th>
          <th class="px-1.5 py-2 text-right font-semibold">Size</th>
          <th class="px-1.5 py-2 text-right font-semibold">Peers</th>
          <th class="px-1.5 py-2 text-right font-semibold">Score</th>
          <th class="py-2"><span class="sr-only">Download</span></th>
        </tr>
      </thead>
      <tbody>
        {#each shown as release (`${release.indexer_id}:${release.guid}`)}
          <tr
            class="border-b border-line align-top last:border-b-0"
            data-release-rejected={!release.approved || undefined}
          >
            <td class="min-w-0 py-2.5 pr-2">
              <p
                class={[
                  'mb-0 truncate text-[11.5px]',
                  !release.approved && 'text-muted',
                ]}
                title={release.title}
              >
                {release.title}
              </p>
              <p
                class="mt-1 mb-0 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[10px] text-muted"
              >
                {#if release === best}<span
                    class="border border-line-strong px-1.5 pt-0.5 pb-px text-[8.5px] font-bold tracking-[0.12em] text-accent uppercase"
                    >Best match</span
                  >{/if}{#if release.full_season}<span>Season pack</span
                  >{/if}{#if release.indexer}<span>{release.indexer}</span
                  >{/if}{#if release.protocol}<span class="capitalize"
                    >{release.protocol}</span
                  >{/if}{#if release.published}<span
                    >{published(release.published)}</span
                  >{/if}
              </p>
              {#each release.rejections as reason (reason)}<p
                  class="mt-1 mb-0 flex items-center gap-1.5 text-[10px] text-warning"
                >
                  <TriangleAlert size={11} class="shrink-0" />{reason}
                </p>{/each}
            </td>
            <td class="px-1.5 py-2.5"
              >{#if release.quality}<span
                  class="inline-block border border-line px-1.5 py-px text-[10px] whitespace-nowrap"
                  >{release.quality}</span
                >{/if}</td
            >
            <td class="px-1.5 py-2.5 text-right whitespace-nowrap tabular-nums"
              >{release.size ? formatBytes(release.size) : ''}</td
            >
            <td class="px-1.5 py-2.5 text-right tabular-nums"
              >{release.protocol === 'torrent'
                ? (release.seeders ?? '')
                : '–'}</td
            >
            <td
              class={[
                'px-1.5 py-2.5 text-right tabular-nums',
                (release.score ?? 0) > 0 && 'text-success',
                (release.score ?? 0) < 0 && 'text-danger',
              ]}
              >{release.score
                ? `${release.score > 0 ? '+' : ''}${release.score}`
                : '0'}</td
            >
            <td class="py-2 text-right">
              {#if confirming === release.guid}<Button
                  variant="danger"
                  size="sm"
                  loading={grabbing === release.guid}
                  aria-label={`Download anyway: ${release.title}`}
                  disabled={grabbing !== null}
                  onclick={() => void grab(release)}>Confirm</Button
                >{:else}<Button
                  variant={release === best
                    ? 'default'
                    : release.approved
                      ? 'secondary'
                      : 'ghost'}
                  size="sm"
                  class="w-8 px-0"
                  aria-label={`${release.approved ? 'Download' : 'Download anyway'}: ${release.title}`}
                  title={release.approved ? 'Download' : 'Download anyway'}
                  loading={grabbing === release.guid}
                  disabled={grabbing !== null}
                  onclick={() => void grab(release)}
                  ><Download size={13} /></Button
                >{/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="mt-3 mb-0 text-[10.5px] text-muted">
      {releases.length}
      {releases.length === 1 ? 'release' : 'releases'}{hidden
        ? ` · ${hidden} rejected hidden`
        : ''}
    </p>
  {/if}
</Modal>
