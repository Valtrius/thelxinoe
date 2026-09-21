<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { rowClass, sectionHeadingClass, statsClass } from './ui/styles';
  type Usage = { bytes: number; free_bytes: number; partial: boolean };
  type Dashboard = {
    storage: { state: Usage; cache: Usage };
    support: {
      items: {
        id: string;
        kind: string;
        problem: boolean;
        unavailable?: boolean;
        rate?: number;
        paused?: boolean;
        remaining_mb?: number;
        queue_count?: number;
        health_count?: number;
      }[];
      checked_at?: number;
    };
    playback: {
      id: string;
      user: string;
      title: string;
      mode: string;
      state: string;
    }[];
    errors: { id: string; kind: string; at: number }[];
    services: {
      kind: string;
      version: string;
      healthy: boolean;
      checked_at: number;
    }[];
  };
  let data = $state<Dashboard | null>(null),
    error = $state(''),
    busy = $state(false);
  let devices = $state<
    { id: string; user: string; name: string; transport: string }[]
  >([]);
  async function load() {
    if (busy) return;
    busy = true;
    try {
      data = await api<Dashboard>('/admin/operations');
      devices = (await api<{ items: typeof devices }>('/admin/devices')).items;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  onMount(() => {
    void load();
    const timer = setInterval(() => void load(), 30000);
    return () => clearInterval(timer);
  });
  const size = (n: number) => `${(n / 1024 ** 3).toFixed(2)} GB`;
  async function diagnostics() {
    try {
      const value = await api('/admin/diagnostics');
      const link = document.createElement('a');
      link.href = URL.createObjectURL(
        new Blob([JSON.stringify(value, null, 2)], {
          type: 'application/json',
        }),
      );
      link.download = 'thelxinoe-diagnostics.json';
      link.click();
      setTimeout(() => URL.revokeObjectURL(link.href), 1000);
    } catch (e) {
      error = String(e);
    }
  }
</script>

<Panel aria-label="Administration overview">
  <div class={sectionHeadingClass}>
    <h2>Administration overview</h2>
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void load()}>Refresh health</Button
    >
  </div>
  {#if error}<p role="alert">{error}</p>{/if}
  {#if data}
    <div class={statsClass}>
      {#each ['state', 'cache'] as key (key)}{@const usage =
          data.storage[key as 'state' | 'cache']}
        <div>
          <strong>{size(usage.bytes)}{usage.partial ? ' +' : ''}</strong><small
            >{key} used · {size(usage.free_bytes)} free</small
          >
        </div>{/each}
    </div>
    <h3>Integrations</h3>
    {#each data.services as service (service.kind)}<div class={rowClass}>
        <strong>{service.kind}</strong><span
          >{service.healthy ? 'Last check passed' : 'Needs attention'} · {new Date(
            service.checked_at * 1000,
          ).toLocaleString()}</span
        >
      </div>{:else}<p>No integrations configured.</p>{/each}
    <h3>Downloads and indexers</h3>
    {#each data.support.items as service (service.id)}<div class={rowClass}>
        <strong>{service.kind}</strong><span
          >{service.unavailable
            ? 'Unavailable'
            : service.problem
              ? 'Needs attention'
              : service.kind === 'nzbget'
                ? `${service.queue_count ?? 0} queued · ${((service.rate ?? 0) / 1024 ** 2).toFixed(2)} MB/s${service.paused ? ' · Paused' : ''}`
                : 'Health check passed'}</span
        >
      </div>{:else}<p>No download or indexer services configured.</p>{/each}
    <h3>Active playback and transcodes</h3>
    {#each data.playback as session (session.id)}<div class={rowClass}>
        <strong>{session.title}</strong><span
          >{session.user} · {session.mode} · {session.state}</span
        >
      </div>{:else}<p>No active playback.</p>{/each}
    <h3>Recent failures</h3>
    {#each data.errors as item (item.id)}<div class={rowClass}>
        <strong>{item.kind}</strong><span
          >{new Date(item.at * 1000).toLocaleString()}</span
        >
      </div>{:else}<p>No failed jobs.</p>{/each}
  {/if}
  <Button variant="secondary" size="form" onclick={() => void diagnostics()}
    >Export redacted diagnostics</Button
  >
  <p class="text-muted">
    Diagnostics include version, database checks and job counts. Credentials,
    paths and viewing history are excluded.
  </p>
  <h3>All devices</h3>
  {#each devices as device (device.id)}<div class={rowClass}>
      <div>
        <strong>{device.user} · {device.name}</strong><small
          >{device.transport}</small
        >
      </div>
      <Button
        variant="secondary"
        size="form"
        onclick={async () => {
          try {
            await api(`/auth/sessions/${device.id}`, 'DELETE');
            await load();
          } catch (e) {
            error = String(e);
          }
        }}>Revoke access</Button
      >
    </div>{/each}
</Panel>
