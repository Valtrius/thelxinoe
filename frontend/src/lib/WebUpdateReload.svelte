<script lang="ts">
  import { onMount } from 'svelte';
  import { api, desktop } from './api';
  import { version as clientVersion } from '../../package.json';
  import {
    requestedWebVersion,
    clearRequestedWebVersion,
    webUpdateSettled,
    type ServerUpdateStatus,
  } from './server-updates';
  import { dismissToastByKey, showToast } from './providers/toasts';
  let { playing = false, admin = false } = $props<{
    playing?: boolean;
    admin?: boolean;
  }>();
  let webVersion = $state('');
  // An open page failed to load code the server no longer has.
  let codeMissing = $state(false);
  // Each server version gets one automatic reload per tab. Still running the
  // old build afterwards (e.g. a proxy cached index.html) asks the person instead.
  const attemptKey = 'thelxinoe:web-reload';
  const stalled = $derived(
    !desktop &&
      Boolean(webVersion) &&
      sessionStorage.getItem(attemptKey) === webVersion,
  );
  onMount(() => {
    if (!desktop && requestedWebVersion() === clientVersion)
      clearRequestedWebVersion();
    if (!desktop && sessionStorage.getItem(attemptKey) === clientVersion)
      sessionStorage.removeItem(attemptKey);
    const changed = (event: Event) =>
      (webVersion = (event as CustomEvent<string>).detail);
    const missing = (event: Event) => {
      changed(event);
      codeMissing = true;
    };
    window.addEventListener('thelxinoe-web-update', changed);
    window.addEventListener('thelxinoe-web-code-missing', missing);
    return () => {
      window.removeEventListener('thelxinoe-web-update', changed);
      window.removeEventListener('thelxinoe-web-code-missing', missing);
    };
  });
  $effect(() => {
    if (desktop || playing || !webVersion || stalled) return;
    let cancelled = false,
      loading = false;
    const reconnect = async () => {
      if (loading) return;
      loading = true;
      try {
        const ready = admin
          ? await api<ServerUpdateStatus>('/admin/product-update').then(
              (status) => webUpdateSettled(status, webVersion),
            )
          : await api<{ version: string }>('/health').then(
              (status) => status.version === webVersion,
            );
        if (!cancelled && ready) {
          sessionStorage.setItem(attemptKey, webVersion);
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
  // Playback defers the reload, so a page that cannot load offers it instead.
  $effect(() => {
    if (!stalled && !(playing && codeMissing && webVersion)) return;
    showToast({
      key: 'web-update',
      title: 'Thelxinoe was updated',
      message: stalled
        ? `This tab still runs version ${clientVersion}. Reload to use ${webVersion}.`
        : `This page needs version ${webVersion}. Reload now, or it reloads when playback stops.`,
      actionLabel: 'Reload',
      onAction: () => location.reload(),
      durationMs: null,
    });
    return () => dismissToastByKey('web-update');
  });
</script>
