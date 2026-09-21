<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { inlineFormClass, panelClass } from './ui/styles';
  type Service = {
    id: string;
    name: string;
    kind: string;
    version: string;
    native_url: string;
  };
  type Download = {
    id: number;
    title: string;
    status: string;
    remaining_mb: number;
    size_mb: number;
  };
  type Missing = {
    movie_id: number | null;
    series_id: number | null;
    episode_id: number | null;
    title: string;
    missing: unknown[];
  };
  type Snapshot = {
    health?: ({ message: string; type: string } | string)[];
    indexers?: {
      id: number;
      name: string;
      enabled: boolean;
      disabled_until: string | null;
    }[];
    queue?: Download[];
    history?: Download[];
    rate?: number;
    limit?: number;
    paused?: boolean;
    free_mb?: number;
    movies?: Missing[];
    episodes?: Missing[];
  };
  let services = $state<Service[]>([]),
    containers = $state<{ id: string; names: string[] }[]>([]),
    snapshots = $state<Record<string, Snapshot>>({}),
    busy = $state(false),
    message = $state('');
  let name = $state(''),
    kind = $state('bazarr'),
    container = $state(''),
    port = $state(6767),
    username = $state(''),
    secret = $state(''),
    native = $state(''),
    limit = $state(0),
    language = $state('en'),
    forced = $state(false),
    hearing = $state(false);
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
  async function load() {
    services = (await api<{ items: Service[] }>('/admin/support')).items;
    containers = (
      await api<{ items: typeof containers }>('/admin/managers/containers')
    ).items;
  }
  async function refresh(id: string) {
    snapshots[id] = await api<Snapshot>(`/admin/support/${id}`);
  }
  async function command(
    id: string,
    action: string,
    extra: Record<string, unknown> = {},
  ) {
    await api(`/admin/support/${id}`, 'POST', { action, ...extra });
    await refresh(id);
    message = 'Service command accepted.';
  }
  async function register() {
    await api('/admin/support', 'POST', {
      name,
      kind,
      container_id: container,
      port,
      credentials: { username, secret },
      native_url: native,
    });
    secret = '';
    await load();
    message = 'Service connected.';
  }
  onMount(() => {
    void act(load);
  });
</script>

<Panel>
  <h2>Subtitles, indexers and downloads</h2>
  <p>
    Connect Bazarr, Prowlarr and NZBGet. Use their own interfaces for advanced
    configuration.
  </p>
  {#if message}<p role="status">{message}</p>{/if}
  <form
    class={inlineFormClass}
    onsubmit={(e) => {
      e.preventDefault();
      void act(register);
    }}
  >
    <label
      >Service name<input bind:value={name} required maxlength="100" /></label
    >
    <label
      >Service type<select
        bind:value={kind}
        onchange={() =>
          (port = kind === 'bazarr' ? 6767 : kind === 'prowlarr' ? 9696 : 6789)}
        ><option value="bazarr">Bazarr</option><option value="prowlarr"
          >Prowlarr</option
        ><option value="nzbget">NZBGet</option></select
      ></label
    >
    <label
      >Service container<select bind:value={container} required
        ><option value="">Select container</option
        >{#each containers as c (c.id)}<option value={c.id}>{c.names[0]}</option
          >{/each}</select
      ></label
    >
    <label
      >Service internal port<input
        type="number"
        bind:value={port}
        min="1"
        max="65535"
        required
      /></label
    >
    {#if kind === 'nzbget'}<label
        >NZBGet username<input
          bind:value={username}
          required
          autocomplete="off"
        /></label
      >{/if}
    <label
      >{kind === 'nzbget' ? 'NZBGet password' : 'Service API key'}<input
        type="password"
        bind:value={secret}
        required
        autocomplete="new-password"
      /></label
    >
    <label
      >Native service UI address<input
        type="url"
        bind:value={native}
        placeholder="https://service.example.com"
      /></label
    >
    <Button type="submit" size="form" disabled={busy}
      >Connect support service</Button
    >
  </form>
  {#each services as service (service.id)}
    {@const data = snapshots[service.id]}
    <article class={panelClass}>
      <h3>{service.name}</h3>
      <p>{service.kind} {service.version}</p>
      {#if service.native_url}<a
          href={service.native_url}
          target="_blank"
          rel="noopener noreferrer">Open {service.name}</a
        >{/if}
      <Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() => void act(() => refresh(service.id))}
        >Refresh {service.name}</Button
      >
      {#if data}
        {#each data.health ?? [] as issue, index (index)}<p>
            {typeof issue === 'string' ? issue : issue.message}
          </p>{/each}
        {#if service.kind === 'prowlarr'}
          {#if !data.indexers?.length}<p>
              No indexers configured. Open Prowlarr to add one.
            </p>{/if}
          {#each data.indexers ?? [] as indexer (indexer.id)}<article>
              <p>
                {indexer.name} · {indexer.enabled
                  ? 'Enabled'
                  : 'Disabled'}{indexer.disabled_until
                  ? ` · unavailable until ${indexer.disabled_until}`
                  : ''}
              </p>
              <Button
                variant="secondary"
                size="form"
                disabled={busy}
                onclick={() =>
                  void act(() =>
                    command(service.id, 'test', { item_id: indexer.id }),
                  )}>Test {indexer.name}</Button
              >
              <Button
                variant="secondary"
                size="form"
                disabled={busy}
                onclick={() =>
                  void act(() =>
                    command(
                      service.id,
                      indexer.enabled ? 'disable' : 'enable',
                      { item_id: indexer.id },
                    ),
                  )}
                >{indexer.enabled ? 'Disable' : 'Enable'} {indexer.name}</Button
              >
            </article>{/each}
        {:else if service.kind === 'nzbget'}
          <p>
            {data.paused ? 'Downloads paused' : 'Downloads running'} · {(
              (data.rate ?? 0) / 1024
            ).toFixed(0)} KB/s · {((data.free_mb ?? 0) / 1024).toFixed(1)} GB free
          </p>
          <Button
            variant="secondary"
            size="form"
            disabled={busy}
            onclick={() =>
              void act(() =>
                command(service.id, data.paused ? 'resume_all' : 'pause_all'),
              )}
            >{data.paused
              ? 'Resume all downloads'
              : 'Pause all downloads'}</Button
          >
          <label
            >Download limit (KB/s; 0 is unlimited)<input
              type="number"
              min="0"
              max="1000000"
              bind:value={limit}
            /></label
          >
          <Button
            variant="secondary"
            size="form"
            disabled={busy}
            onclick={() =>
              void act(() => command(service.id, 'rate', { value: limit }))}
            >Set download limit</Button
          >
          {#each data.queue ?? [] as download (download.id)}<article>
              <p>
                {download.title} · {download.status} · {download.remaining_mb} / {download.size_mb}
                MB remaining
              </p>
              <Button
                variant="secondary"
                size="form"
                disabled={busy}
                onclick={() =>
                  void act(() =>
                    command(service.id, 'pause', { item_id: download.id }),
                  )}>Pause {download.title}</Button
              >
              <Button
                variant="secondary"
                size="form"
                disabled={busy}
                onclick={() =>
                  void act(() =>
                    command(service.id, 'resume', { item_id: download.id }),
                  )}>Resume {download.title}</Button
              >
              <Button
                variant="secondary"
                size="form"
                disabled={busy}
                onclick={() =>
                  void act(() =>
                    command(service.id, 'remove', { item_id: download.id }),
                  )}>Remove {download.title}</Button
              >
            </article>{/each}
          <h4>Recent downloads</h4>
          {#each data.history ?? [] as download (download.id)}<p>
              {download.title} · {download.status}
            </p>{/each}
        {:else}
          <label
            >Subtitle language code<input
              bind:value={language}
              minlength="2"
              maxlength="3"
              placeholder="en"
            /></label
          >
          <Switch bind:checked={forced}>Forced subtitles</Switch><Switch
            bind:checked={hearing}>Hearing impaired subtitles</Switch
          >
          {#if !data.movies?.length && !data.episodes?.length}<p>
              No missing subtitles reported. Configure language profiles and
              providers in Bazarr.
            </p>{/if}
          {#each [...(data.movies ?? []), ...(data.episodes ?? [])] as item (`${item.movie_id}:${item.episode_id}`)}<p
            >
              {item.title}
            </p>
            <Button
              variant="secondary"
              size="form"
              disabled={busy}
              onclick={() =>
                void act(() =>
                  command(service.id, 'subtitles', {
                    item_id: item.movie_id ?? item.episode_id,
                    domain: item.movie_id ? 'movies' : 'episodes',
                    language,
                    forced,
                    hearing_impaired: hearing,
                  }),
                )}>Find subtitles for {item.title}</Button
            >{/each}
        {/if}
      {/if}
    </article>
  {/each}
</Panel>
