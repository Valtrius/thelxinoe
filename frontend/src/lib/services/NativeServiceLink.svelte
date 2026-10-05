<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { ExternalLink } from '@lucide/svelte';
  import { desktop, serverUrl } from '../api';

  let { id, label, path } = $props<{
    id: string;
    label: string;
    path: string;
  }>();
  let error = $state('');
  async function open(event: MouseEvent) {
    if (!desktop) return;
    event.preventDefault();
    error = '';
    try {
      await invoke('open_service', { id });
    } catch (caught) {
      error = String(caught);
    }
  }
</script>

<div class="min-w-0">
  <a
    class="service-link flex w-fit max-w-full items-center gap-1.25 text-[11px] text-accent"
    href={/^https?:\/\//.test(path) ? path : `${serverUrl()}${path}`}
    target="_blank"
    rel="noopener noreferrer"
    onclick={open}
    aria-label={`Open ${label}`}
    ><span class="min-w-0 truncate">Open {label}</span><ExternalLink
      size={13}
      class="shrink-0"
      aria-hidden="true"
    /></a
  >
  {#if error}<p class="mt-1 text-[11px] text-muted" role="alert">
      {error}
    </p>{/if}
</div>
