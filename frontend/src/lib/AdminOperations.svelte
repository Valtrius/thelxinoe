<script lang="ts">
  let { timezone, timeFormat } = $props<{
    timezone: string;
    timeFormat: '12h' | '24h';
  }>();
  import { onMount } from 'svelte';
  import { LoaderCircle } from '@lucide/svelte';
  import { api } from './api';
  import { isServerUpdateInterruption } from './server-updates';
  import Button from './ui/Button.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import {
    cardBlockClass,
    groupHeadingClass,
    statGridClass,
    statTileClass,
  } from './ui/styles';
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
      const [dashboard, connected] = await Promise.all([
        api<Dashboard>('/admin/operations'),
        api<{ items: typeof devices }>('/admin/devices'),
      ]);
      data = dashboard;
      devices = connected.items;
      error = '';
    } catch (e) {
      if (!isServerUpdateInterruption(e)) error = String(e);
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
  const when = (seconds: number) =>
    new Date(seconds * 1000).toLocaleString(undefined, {
      timeZone: timezone,
      hour12: timeFormat === '12h',
    });
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

<SettingsCard
  id="server-overview"
  aria-label="Administration overview"
  aria-busy={busy}
  title="Administration overview"
  description="Storage, integrations and what the server is doing right now. Refreshes every 30 seconds."
>
  {#snippet actions()}
    {#if busy}
      <span
        role="status"
        aria-label="Loading administration overview"
        class="text-muted"
      >
        <LoaderCircle
          size={16}
          class="animate-spin motion-reduce:animate-none"
          aria-hidden="true"
        />
        <span class="sr-only">Loading administration overview</span>
      </span>
    {/if}
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void load()}>Refresh health</Button
    >
  {/snippet}
  {#if error}<p role="alert" class="m-0 {cardBlockClass} text-xs text-danger">
      {error}
    </p>{/if}
  {#if data}
    <section>
      <h3 class={groupHeadingClass}>Storage</h3>
      <div class="{statGridClass} pt-2">
        {#each ['state', 'cache'] as key (key)}{@const usage =
            data.storage[key as 'state' | 'cache']}
          <div class={statTileClass}>
            <strong>{size(usage.bytes)}{usage.partial ? ' +' : ''}</strong
            ><small>{key} used · {size(usage.free_bytes)} free</small>
          </div>{/each}
      </div>
    </section>
    <section>
      <h3 class={groupHeadingClass}>Integrations</h3>
      <div class="divide-y divide-line">
        {#each data.services as service (service.kind)}
          <SettingRow
            label={service.kind}
            description={`${service.healthy ? 'Last check passed' : 'Needs attention'} · ${when(service.checked_at)}`}
          >
            <StatusChip tone={service.healthy ? 'ok' : 'bad'}
              >{service.healthy ? 'Healthy' : 'Attention'}</StatusChip
            >
          </SettingRow>
        {:else}<p class="m-0 {cardBlockClass} pt-2 text-xs text-muted">
            No integrations configured.
          </p>{/each}
      </div>
    </section>
    <section>
      <h3 class={groupHeadingClass}>Downloads and indexers</h3>
      <div class="divide-y divide-line">
        {#each data.support.items as service (service.id)}
          <SettingRow
            label={service.kind}
            description={service.unavailable
              ? 'Unavailable'
              : service.problem
                ? 'Needs attention'
                : service.kind === 'nzbget'
                  ? `${service.queue_count ?? 0} queued · ${((service.rate ?? 0) / 1024 ** 2).toFixed(2)} MB/s${service.paused ? ' · Paused' : ''}`
                  : 'Health check passed'}
          >
            <StatusChip
              tone={service.unavailable
                ? 'neutral'
                : service.problem
                  ? 'bad'
                  : service.paused
                    ? 'warn'
                    : 'ok'}
              >{service.unavailable
                ? 'Unavailable'
                : service.problem
                  ? 'Attention'
                  : service.paused
                    ? 'Paused'
                    : 'Healthy'}</StatusChip
            >
          </SettingRow>
        {:else}<p class="m-0 {cardBlockClass} pt-2 text-xs text-muted">
            No download or indexer services configured.
          </p>{/each}
      </div>
    </section>
    <section>
      <h3 class={groupHeadingClass}>Active playback and transcodes</h3>
      <div class="divide-y divide-line">
        {#each data.playback as session (session.id)}
          <SettingRow
            label={session.title}
            description={`${session.user} · ${session.mode} · ${session.state}`}
          />
        {:else}<p class="m-0 {cardBlockClass} pt-2 text-xs text-muted">
            No active playback.
          </p>{/each}
      </div>
    </section>
    <section>
      <h3 class={groupHeadingClass}>Recent failures</h3>
      <div class="divide-y divide-line">
        {#each data.errors as item (item.id)}
          <SettingRow label={item.kind} description={when(item.at)}>
            <StatusChip tone="bad">Failed</StatusChip>
          </SettingRow>
        {:else}<p class="m-0 {cardBlockClass} pt-2 text-xs text-muted">
            No failed jobs.
          </p>{/each}
      </div>
    </section>
  {/if}
</SettingsCard>
<SettingsCard id="server-diagnostics" title="Diagnostics">
  <SettingRow
    label="Export redacted diagnostics"
    description="Diagnostics include version, database checks and job counts. Credentials, paths and viewing history are excluded."
  >
    <Button variant="secondary" size="form" onclick={() => void diagnostics()}
      >Export redacted diagnostics</Button
    >
  </SettingRow>
</SettingsCard>
<SettingsCard
  id="server-devices"
  title="All devices"
  description="Sessions across every user on this server."
>
  {#each devices as device (device.id)}
    <SettingRow
      label={`${device.user} · ${device.name}`}
      description={device.transport}
    >
      <Button
        variant="ghost"
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
    </SettingRow>
  {:else}
    <p class="m-0 {cardBlockClass} text-xs text-muted">
      {data ? 'No connected devices.' : 'Loading devices…'}
    </p>
  {/each}
</SettingsCard>
