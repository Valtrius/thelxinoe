<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  type Tools = {
    installed: boolean;
    yt_dlp: string | null;
    deno: string | null;
    job: { id: string; state: string; error: string | null } | null;
  };
  let tools = $state<Tools | null>(null),
    error = $state(''),
    busy = $state(false);
  let downloads = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined,
    disposed = false;
  const installing = $derived(
    busy || ['queued', 'running'].includes(tools?.job?.state ?? ''),
  );
  async function load() {
    try {
      tools = await api<Tools>('/admin/online/tools');
      downloads = (await api<{ enabled: boolean }>('/admin/online/downloads'))
        .enabled;
      error = '';
    } catch (e) {
      error = String(e);
    }
    clearTimeout(timer);
    if (!disposed && ['queued', 'running'].includes(tools?.job?.state ?? ''))
      timer = setTimeout(() => void load(), 2000);
  }
  onMount(() => {
    void load();
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  });
  async function install() {
    busy = true;
    error = '';
    try {
      await api('/admin/online/tools', 'POST');
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel">
  <h2>Server video tools</h2>
  <label
    ><input
      type="checkbox"
      bind:checked={downloads}
      onchange={async () => {
        try {
          await api('/admin/online/downloads', 'PUT', { enabled: downloads });
        } catch (e) {
          error = String(e);
          await load();
        }
      }}
    />Allow YouTube downloads</label
  >
  <p class="muted">
    Saved or pinned public videos can be downloaded for playback. Files are
    shared; personal progress stays private. Files with no remaining interest
    are removed after one day.
  </p>
  <p class="muted">
    yt-dlp extracts public YouTube media. Deno runs its JavaScript support.
    Downloads come from the official releases and are checked against their
    published SHA-256 digests.
  </p>
  {#if tools?.installed}<p>
      yt-dlp {tools.yt_dlp} · Deno {tools.deno}
    </p>{:else}<p class="muted">
      The server tools have not been installed.
    </p>{/if}
  <button class="secondary" disabled={installing || !tools} onclick={install}
    >{installing
      ? 'Installing server tools…'
      : tools?.installed
        ? 'Update server tools'
        : 'Install server tools'}</button
  >
  {#if tools?.job?.error}<p role="status">{tools.job.error}</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>
