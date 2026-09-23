<script lang="ts" module>
  export type ServiceConnection = {
    id: string;
    source_id: string;
    target_id: string;
    source_kind: string;
    target_kind: string;
    target_name: string;
    kind: string;
    enabled: boolean;
    state: string;
    error: string | null;
    cleanup_pending: boolean;
  };
</script>

<script lang="ts">
  import Button from './ui/Button.svelte';
  let {
    kind,
    connections,
    busy,
    onaction,
  }: {
    kind: string;
    connections: ServiceConnection[];
    busy: boolean;
    onaction: (
      connection: ServiceConnection,
      action: 'connect' | 'disconnect' | 'retry',
    ) => void;
  } = $props();
  const titles: Record<string, string> = {
    prowlarr: 'Applications',
    bazarr: 'Media managers',
    radarr: 'Download clients',
    sonarr: 'Download clients',
    lidarr: 'Download clients',
  };
  const items = $derived(
    connections.filter((connection) => connection.source_kind === kind),
  );
  function status(connection: ServiceConnection) {
    if (connection.cleanup_pending)
      return connection.state === 'conflict'
        ? 'Disconnect needs attention'
        : 'Disconnect pending';
    if (!connection.enabled)
      return connection.state === 'available'
        ? 'Available to connect'
        : 'Not connected';
    return (
      (
        {
          pending: 'Connecting',
          connected: 'Connected',
          unavailable: 'Connection unavailable',
          conflict: 'Connection needs attention',
        } as Record<string, string>
      )[connection.state] ?? 'Connecting'
    );
  }
</script>

{#if titles[kind]}
  <section aria-label={titles[kind]} class="border-t border-line py-3">
    <h3 class="text-xs font-semibold">{titles[kind]}</h3>
    {#if items.length === 0}
      <p class="text-muted">
        No compatible services are connected to Thelxinoe yet.
      </p>
    {/if}
    {#each items as connection (connection.id)}
      <div
        class="connection-row"
        aria-label={`${connection.target_name} connection`}
      >
        <div class="flex flex-wrap items-center justify-between gap-2">
          <strong class="text-xs">{connection.target_name}</strong>
          <span class="text-xs text-muted" role="status"
            >{status(connection)}</span
          >
        </div>
        {#if connection.error && (connection.enabled || connection.cleanup_pending)}
          <p class="text-xs" role="status">{connection.error}</p>
        {/if}
        {#if connection.enabled && connection.state === 'unavailable'}
          <p class="text-xs text-muted">
            Retries automatically when both services are available.
          </p>
        {/if}
        {#if connection.cleanup_pending}
          <p class="text-xs text-muted">
            Automatic reconnection is disabled. {connection.state === 'conflict'
              ? 'Resolve the reported configuration change, then retry removal.'
              : 'Removal finishes when the source service is available.'}
          </p>
        {/if}
        <div class="mt-2 flex flex-wrap gap-2">
          {#if connection.enabled}
            <Button
              variant="secondary"
              size="sm"
              disabled={busy}
              onclick={() => onaction(connection, 'disconnect')}
              >Disconnect</Button
            >
          {:else if !connection.cleanup_pending}
            <Button
              variant="secondary"
              size="sm"
              disabled={busy}
              onclick={() => onaction(connection, 'connect')}>Connect</Button
            >
          {/if}
          {#if ['unavailable', 'conflict'].includes(connection.state) && (connection.enabled || connection.cleanup_pending)}
            <Button
              variant="secondary"
              size="sm"
              disabled={busy}
              onclick={() => onaction(connection, 'retry')}>Retry</Button
            >
          {/if}
        </div>
      </div>
    {/each}
  </section>
{/if}

<style>
  section > h3 {
    margin-bottom: 12px;
  }
  section p {
    font-size: 11px;
    line-height: 1.6;
    margin: 8px 0;
  }
  .connection-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 8px 16px;
    padding: 12px 0;
    border-bottom: 1px solid var(--line);
  }
  .connection-row > div:first-child {
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 4px;
  }
  .connection-row > div:last-child {
    grid-column: 2;
    grid-row: 1;
    align-items: center;
    margin: 0;
  }
  .connection-row > p {
    grid-column: 1/-1;
    margin: 0;
  }
  .connection-row:last-child {
    border-bottom: 0;
  }
</style>
