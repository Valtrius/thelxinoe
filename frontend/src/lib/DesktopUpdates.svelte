<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  let installed = $state(''),
    release = $state<{ version: string; notes: string; bytes: number } | null>(
      null,
    ),
    busy = $state(false),
    message = $state(''),
    progress = $state(0);
  async function check() {
    busy = true;
    message = '';
    try {
      const value = await invoke<{
        installed: string;
        release: typeof release;
      }>('desktop_update_check');
      installed = value.installed;
      release = value.release;
      if (!release)
        message =
          'No newer signed desktop release is available from this server.';
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
  async function install() {
    busy = true;
    message = 'Downloading and verifying the signed installer…';
    try {
      await invoke('desktop_update_install');
    } catch (e) {
      message = String(e);
      busy = false;
    }
  }
  onMount(() => {
    const subscription = listen<{ received: number; total: number }>(
      'desktop-update-progress',
      (e) => {
        progress = e.payload.total
          ? Math.min(100, (100 * e.payload.received) / e.payload.total)
          : 0;
      },
    );
    return () => {
      void subscription.then((stop) => stop());
    };
  });
</script>

<Panel aria-label="Desktop updates">
  <h2>Windows updates</h2>
  {#if installed}<p>Installed desktop: {installed}</p>{/if}
  <Button variant="secondary" size="form" disabled={busy} onclick={check}
    >Check desktop release</Button
  >
  {#if release}<h3>Version {release.version}</h3>
    <p>{release.notes}</p>
    <p>
      Installation closes Thelxinoe and stops desktop playback, then reopens the
      updated application.
    </p>
    <Button size="form" disabled={busy} onclick={install}
      >Download and install desktop update</Button
    >{/if}
  {#if busy && progress > 0}<progress
      value={progress}
      max="100"
      aria-label="Desktop update download"
    ></progress>{/if}
  {#if message}<p role="status">{message}</p>{/if}
</Panel>
