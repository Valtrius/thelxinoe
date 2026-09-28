<script lang="ts">
  import { onMount } from 'svelte';
  import { desktop } from './api';
  import { desktopUpdates } from './desktop-updates';
  import DesktopUpdates from './DesktopUpdates.svelte';
  import Button from './ui/Button.svelte';
  import Modal from './ui/Modal.svelte';
  let { playing = false } = $props<{ playing?: boolean }>();
  let open = $state(false);
  let dismissed = $state('');
  let webVersion = $state('');
  onMount(() => {
    const changed = (event: Event) =>
      (webVersion = (event as CustomEvent<string>).detail);
    window.addEventListener('thelxinoe-web-update', changed);
    return () => window.removeEventListener('thelxinoe-web-update', changed);
  });
  const version = $derived(
    desktop ? $desktopUpdates?.release?.version : webVersion,
  );
</script>

{#if version && version !== dismissed}
  <aside
    aria-label="Client update available"
    class="fixed right-4 bottom-4 z-70 grid max-w-[min(26rem,calc(100vw-2rem))] gap-3 border border-line-strong bg-surface-strong p-4 text-sm shadow-panel"
  >
    <p>{desktop ? 'Desktop' : 'Web app'} {version} is available.</p>
    <div class="flex flex-wrap gap-2">
      {#if desktop}<Button size="sm" onclick={() => (open = true)}
          >View desktop update</Button
        >
      {:else}<Button
          size="sm"
          disabled={playing}
          onclick={() => location.reload()}>Reload web app</Button
        >{/if}
      <Button
        size="sm"
        variant="secondary"
        onclick={() => (dismissed = version)}>Later</Button
      >
    </div>
    {#if !desktop && playing}<p class="text-xs text-muted">
        Finish playback before reloading.
      </p>{/if}
  </aside>
{/if}
{#if open}<Modal title="Desktop update" onClose={() => (open = false)}
    ><DesktopUpdates /></Modal
  >{/if}
