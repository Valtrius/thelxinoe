<script lang="ts">
  import { api } from './api';
  let { id, admin } = $props<{ id: string; admin: boolean }>();
  type Status = {
    available: boolean;
    monitored: boolean;
    downloads: {
      title: string;
      status: string;
      size: number;
      remaining: number;
      time_left: string;
    }[];
  };
  type Release = {
    guid: string;
    indexer_id: number;
    title: string;
    size: number;
    score: number | null;
    approved: boolean;
    rejections: string[];
  };
  let status = $state<Status | null>(null),
    releases = $state<Release[]>([]),
    busy = $state(false),
    message = $state('');
  async function act(fn: () => Promise<void>) {
    busy = true;
    message = '';
    try {
      await fn();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
  async function refresh() {
    status = await api<Status>(`/acquisition/requests/${id}/status`);
  }
</script>

<button disabled={busy} onclick={() => void act(refresh)}
  >Check availability</button
>
{#if message}<p role="status">{message}</p>{/if}
{#if status}
  <p>
    {status.available ? 'Available in manager' : 'Waiting for media'} · {status.monitored
      ? 'Monitored'
      : 'Unmonitored'}
  </p>
  {#each status.downloads as download, index (index)}<p>
      {download.title} · {download.status} · {download.time_left ?? ''}
    </p>{/each}
  {#if admin}
    <button
      disabled={busy}
      onclick={() =>
        void act(async () => {
          await api(`/admin/acquisition/requests/${id}/monitor`, 'PUT', {
            monitored: !status?.monitored,
          });
          await refresh();
        })}>{status.monitored ? 'Unmonitor' : 'Monitor'}</button
    >
    <button
      disabled={busy}
      onclick={() =>
        void act(async () => {
          releases = (
            await api<{ items: Release[] }>(
              `/admin/acquisition/requests/${id}/releases`,
            )
          ).items;
          if (!releases.length)
            message =
              'The manager returned no releases. Check its indexers and download settings.';
        })}>Search releases</button
    >
  {/if}
{/if}
{#each releases as release (`${release.indexer_id}:${release.guid}`)}
  <article class="panel">
    <strong>{release.title}</strong>
    <p>
      Manager score: {release.score ?? 'not supplied'} · {(
        release.size / 1e9
      ).toFixed(2)} GB
    </p>
    {#each release.rejections ?? [] as reason, index (index)}<p>
        {reason}
      </p>{/each}
    <button
      disabled={busy ||
        release.approved === false ||
        !!release.rejections?.length}
      onclick={() =>
        void act(async () => {
          await api(`/admin/acquisition/requests/${id}/releases`, 'POST', {
            guid: release.guid,
            indexer_id: release.indexer_id,
          });
          releases = [];
          message = 'Release submitted to the manager.';
          await refresh();
        })}>Grab release</button
    >
  </article>
{/each}
