<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  type Container = {
    id: string;
    names: string[];
    image: string;
    state: string;
  };
  type Defaults = {
    root_folder: string;
    quality_profile: number;
    metadata_profile: number | null;
    monitored: boolean;
  };
  type Service = {
    id: string;
    name: string;
    kind: string;
    version: string;
    defaults: Partial<Defaults>;
    error: string | null;
  };
  type Options = {
    roots: { id: number; path: string }[];
    profiles: { id: number; name: string }[];
    metadata_profiles: { id: number; name: string }[];
  };
  let approvalUsers = $state<
    { id: string; username: string; enabled: boolean }[]
  >([]);
  let containers = $state<Container[]>([]),
    services = $state<Service[]>([]),
    container = $state(''),
    name = $state(''),
    kind = $state('radarr'),
    port = $state(7878),
    key = $state(''),
    busy = $state(false),
    message = $state('');
  let selected = $state<Service | null>(null),
    options = $state<Options | null>(null),
    root = $state(''),
    profile = $state(0),
    metadata = $state<number | null>(null),
    monitored = $state(true);
  async function load() {
    approvalUsers = (
      await api<{ items: typeof approvalUsers }>('/admin/acquisition/users')
    ).items;
    services = (await api<{ items: Service[] }>('/admin/managers')).items;
    containers = (
      await api<{ items: Container[] }>('/admin/managers/containers')
    ).items;
  }
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
  async function register() {
    await api('/admin/managers', 'POST', {
      name,
      kind,
      container_id: container,
      port,
      api_key: key,
    });
    key = '';
    await load();
    message = 'Manager connected. Choose acquisition defaults below.';
  }
  async function edit(s: Service) {
    selected = s;
    options = await api<Options>(`/admin/managers/${s.id}/options`);
    root = s.defaults.root_folder ?? options.roots[0]?.path ?? '';
    profile = s.defaults.quality_profile ?? options.profiles[0]?.id ?? 0;
    metadata =
      s.defaults.metadata_profile ?? options.metadata_profiles[0]?.id ?? null;
    monitored = s.defaults.monitored ?? true;
  }
  async function save() {
    if (!selected) return;
    await api(`/admin/managers/${selected.id}/defaults`, 'PUT', {
      root_folder: root,
      quality_profile: profile,
      metadata_profile: metadata,
      monitored,
    });
    selected = null;
    await load();
    message = 'Acquisition defaults saved.';
  }
  onMount(() => {
    void act(load);
  });
</script>

<section class="panel">
  <h2>Acquisition managers</h2>
  <p>
    Connect local Radarr, Sonarr and Lidarr containers. The controller verifies
    their network and shared media mounts. API keys are stored encrypted.
  </p>
  {#if message}<p role="status">{message}</p>{/if}
  <h3>Automatic request approval</h3>
  <p>
    Administrators can request directly. Enable automatic approval for trusted
    users below.
  </p>
  {#each approvalUsers as user (user.id)}
    <label
      ><input
        type="checkbox"
        checked={user.enabled}
        disabled={busy}
        onchange={(event) => {
          const enabled = event.currentTarget.checked;
          void act(async () => {
            await api(`/admin/acquisition/users/${user.id}`, 'PUT', {
              enabled,
            });
            await load();
          });
        }}
      />
      {user.username}</label
    >
  {/each}
  <form
    class="inline-form"
    onsubmit={(e) => {
      e.preventDefault();
      void act(register);
    }}
  >
    <label
      >Manager name<input bind:value={name} required maxlength="100" /></label
    >
    <label
      >Manager type<select
        bind:value={kind}
        onchange={() =>
          (port = kind === 'radarr' ? 7878 : kind === 'sonarr' ? 8989 : 8686)}
        ><option value="radarr">Radarr</option><option value="sonarr"
          >Sonarr</option
        ><option value="lidarr">Lidarr</option></select
      ></label
    >
    <label
      >Docker container<select bind:value={container} required
        ><option value="">Select container</option
        >{#each containers as c (c.id)}<option value={c.id}
            >{c.names[0]} ({c.state})</option
          >{/each}</select
      ></label
    >
    <label
      >Internal API port<input
        type="number"
        bind:value={port}
        min="1"
        max="65535"
        required
      /></label
    >
    <label
      >Manager API key<input
        type="password"
        bind:value={key}
        required
        autocomplete="new-password"
      /></label
    >
    <button class="primary" disabled={busy}>Connect manager</button>
  </form>
  <button class="secondary" disabled={busy} onclick={() => void act(load)}
    >Refresh containers</button
  >
  {#each services as service (service.id)}<article class="panel">
      <h3>{service.name}</h3>
      <p>{service.kind} {service.version}</p>
      {#if service.error}<p role="status">{service.error}</p>{/if}<button
        class="secondary"
        disabled={busy}
        onclick={() => void act(() => edit(service))}
        >Defaults for {service.name}</button
      ><button
        class="secondary"
        disabled={busy}
        onclick={() =>
          void act(async () => {
            await api(`/admin/managers/${service.id}/test`, 'POST');
            message = 'Connection and mount checks passed.';
          })}>Test {service.name}</button
      >
    </article>{/each}
  {#if selected && options}<form
      class="panel"
      onsubmit={(e) => {
        e.preventDefault();
        void act(save);
      }}
    >
      <h3>Defaults for {selected.name}</h3>
      {#if !options.roots.length}<p>
          Add a root folder in the manager's own settings first.
        </p>{/if}
      <label
        >Acquisition root folder<select bind:value={root} required
          >{#each options.roots as r (r.id)}<option value={r.path}
              >{r.path}{r.id === 0 ? ' (create)' : ''}</option
            >{/each}</select
        ></label
      >
      <label
        >Acquisition quality profile<select bind:value={profile} required
          >{#each options.profiles as p (p.id)}<option value={p.id}
              >{p.name}</option
            >{/each}</select
        ></label
      >
      {#if selected.kind === 'lidarr'}<label
          >Acquisition metadata profile<select bind:value={metadata} required
            >{#each options.metadata_profiles as p (p.id)}<option value={p.id}
                >{p.name}</option
              >{/each}</select
          ></label
        >{/if}
      <label
        ><input type="checkbox" bind:checked={monitored} /> Monitor and search approved
        requests</label
      ><button class="primary" disabled={busy || !root || !profile}
        >Save acquisition defaults</button
      >
    </form>{/if}
</section>
