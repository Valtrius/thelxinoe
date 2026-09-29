<script lang="ts">
  import { onMount } from 'svelte';
  import { api, desktop } from './api';
  import { version as clientVersion } from '../../package.json';
  import {
    requestedWebVersion,
    clearRequestedWebVersion,
    type ServerUpdateStatus,
  } from './server-updates';
  let { playing = false, admin = false } = $props<{
    playing?: boolean;
    admin?: boolean;
  }>();
  let webVersion = $state('');
  onMount(() => {
    if (!desktop && requestedWebVersion() === clientVersion)
      clearRequestedWebVersion();
    const changed = (event: Event) =>
      (webVersion = (event as CustomEvent<string>).detail);
    window.addEventListener('thelxinoe-web-update', changed);
    return () => window.removeEventListener('thelxinoe-web-update', changed);
  });
  $effect(() => {
    if (desktop || playing || !webVersion) return;
    let cancelled = false,
      loading = false;
    const reconnect = async () => {
      if (loading) return;
      loading = true;
      try {
        const ready = admin
          ? await api<ServerUpdateStatus>('/admin/product-update').then(
              (status) =>
                status.version === webVersion &&
                !status.controller.error &&
                (!status.controller.items.length ||
                  status.controller.items.some(
                    (item) =>
                      (item.version === webVersion &&
                        item.stage === 'committed') ||
                      (item.previous_version === webVersion &&
                        ['restored', 'recovered'].includes(item.stage)),
                  )),
            )
          : await api<{ version: string }>('/health').then(
              (status) => status.version === webVersion,
            );
        if (!cancelled && ready) {
          clearRequestedWebVersion();
          location.reload();
        }
      } catch {
        /* Retry while the server restarts. */
      } finally {
        loading = false;
      }
    };
    void reconnect();
    const timer = setInterval(() => void reconnect(), 3000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  });
</script>
