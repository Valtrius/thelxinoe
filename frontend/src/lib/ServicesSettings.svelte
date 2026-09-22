<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import Switch from './ui/Switch.svelte';
  import { badgeClass, sectionHeadingClass } from './ui/styles';
  import { CircleX } from '@lucide/svelte';

  type ServiceKind =
    'radarr' | 'sonarr' | 'lidarr' | 'bazarr' | 'prowlarr' | 'nzbget';
  type Definition = {
    kind: ServiceKind;
    label: string;
    role: 'manager' | 'support';
    internalPort: number;
    hostPort: number;
    description: string;
  };
  type Container = {
    id: string;
    names: string[];
    image?: string;
    state?: string;
  };
  type ManagerDefaults = {
    root_folder: string;
    quality_profile: number;
    metadata_profile: number | null;
    monitored: boolean;
  };
  type ManagerService = {
    id: string;
    name: string;
    kind: ServiceKind;
    container_id: string;
    port: number;
    version: string;
    defaults: Partial<ManagerDefaults>;
    checked_at: number;
    error: string | null;
  };
  type SupportService = {
    id: string;
    name: string;
    kind: ServiceKind;
    version: string;
    native_url: string;
    checked_at: number;
    error?: string | null;
  };
  type ManagerOptions = {
    roots: { id: number; path: string }[];
    profiles: { id: number; name: string }[];
    metadata_profiles: { id: number; name: string }[];
  };
  type SupportDownload = {
    id: number;
    title: string;
    status: string;
    remaining_mb: number;
    size_mb: number;
  };
  type SupportMissing = {
    movie_id: number | null;
    series_id: number | null;
    episode_id: number | null;
    title: string;
    missing: unknown[];
  };
  type SupportSnapshot = {
    health?: ({ message: string; type: string } | string)[];
    indexers?: {
      id: number;
      name: string;
      enabled: boolean;
      disabled_until: string | null;
    }[];
    queue?: SupportDownload[];
    history?: SupportDownload[];
    rate?: number;
    limit?: number;
    paused?: boolean;
    free_mb?: number;
    movies?: SupportMissing[];
    episodes?: SupportMissing[];
  };
  type StackService = {
    id: string;
    kind: ServiceKind;
    name: string;
    phase: string;
    image: string;
    drift: boolean;
    running: boolean;
    error: string | null;
    transfer_pending: boolean;
  };
  type Provision = {
    id: string;
    kind: ServiceKind;
    state: string;
    host_port: number;
    container_id: string | null;
    service_id: string | null;
    error: string | null;
    native_url: string;
    origin: string;
  };
  type TransferReview = {
    review_id: string;
    name: string;
    image: string;
    source_config: string;
    managed_config: string;
    compose_project: string | null;
    compose_service: string | null;
  };
  type UpdatePolicy = {
    service_id: string;
    policy: string;
    window_start: number;
    window_end: number;
    candidate: string | null;
    error: string | null;
  };
  type Update = {
    id: string;
    service_id: string;
    state: string;
    candidate: string | null;
    error: string | null;
    classification?: string;
  };
  type UpdateTarget = { id: string; kind: ServiceKind };
  type ServerUpdatePolicy = {
    policy: string;
    window_start: number;
    window_end: number;
  };
  type ApprovalUser = { id: string; username: string; enabled: boolean };
  type SetupDraft = {
    name: string;
    container: string;
    port: number;
    username: string;
    secret: string;
    nativeUrl: string;
  };
  type InstallDraft = { hostPort: number; nativeUrl: string };
  type DefaultsDraft = ManagerDefaults & { loaded: boolean };
  type UpdateDraft = {
    targetId: string | null;
    policy: string;
    start: number;
    end: number;
  };

  const definitions: Definition[] = [
    {
      kind: 'radarr',
      label: 'Radarr',
      role: 'manager',
      internalPort: 7878,
      hostPort: 17878,
      description: 'Movie acquisition and metadata.',
    },
    {
      kind: 'sonarr',
      label: 'Sonarr',
      role: 'manager',
      internalPort: 8989,
      hostPort: 18989,
      description: 'Series acquisition and episode metadata.',
    },
    {
      kind: 'lidarr',
      label: 'Lidarr',
      role: 'manager',
      internalPort: 8686,
      hostPort: 18686,
      description: 'Music acquisition and album metadata.',
    },
    {
      kind: 'bazarr',
      label: 'Bazarr',
      role: 'support',
      internalPort: 6767,
      hostPort: 16767,
      description: 'Subtitle discovery and retrieval.',
    },
    {
      kind: 'prowlarr',
      label: 'Prowlarr',
      role: 'support',
      internalPort: 9696,
      hostPort: 19696,
      description: 'Indexer health and manager wiring.',
    },
    {
      kind: 'nzbget',
      label: 'NZBGet',
      role: 'support',
      internalPort: 6789,
      hostPort: 16789,
      description: 'Download queue and transfer controls.',
    },
  ];
  const stages: Record<string, string> = {
    queued: 'Waiting to check',
    submitting: 'Starting check',
    preparing: 'Preparing',
    snapshotting: 'Saving appdata',
    preflight: 'Checking compatibility',
    ready: 'Compatible, ready to install',
    'queued-activate': 'Waiting to install',
    'recovery-snapshot': 'Saving recovery copy',
    'isolated-live-validation': 'Validating before activation',
    activating: 'Starting updated service',
    committed: 'Installed',
    'rolled-back': 'Previous service restored',
    blocked: 'Needs attention',
    'recovery-required': 'Recovery required',
    'runtime-failure': 'Updated service needs attention',
    'queued-recover': 'Waiting to restore',
  };
  const loaderSubsystems = [
    { key: 'managers', label: 'Acquisition managers' },
    { key: 'support', label: 'Support services' },
    { key: 'approval', label: 'Request approval settings' },
    { key: 'updates', label: 'Service updates' },
    { key: 'containers', label: 'Docker container discovery' },
    { key: 'stack', label: 'Managed service runtime' },
  ] as const;
  const setup = $state(
    Object.fromEntries(
      definitions.map((definition) => [
        definition.kind,
        {
          name: definition.label,
          container: '',
          port: definition.internalPort,
          username: '',
          secret: '',
          nativeUrl: '',
        } satisfies SetupDraft,
      ]),
    ) as Record<ServiceKind, SetupDraft>,
  );
  const installs = $state(
    Object.fromEntries(
      definitions.map((definition) => [
        definition.kind,
        { hostPort: definition.hostPort, nativeUrl: '' } satisfies InstallDraft,
      ]),
    ) as Record<ServiceKind, InstallDraft>,
  );
  const defaults = $state(
    Object.fromEntries(
      definitions
        .filter((definition) => definition.role === 'manager')
        .map((definition) => [
          definition.kind,
          {
            loaded: false,
            root_folder: '',
            quality_profile: 0,
            metadata_profile: null,
            monitored: true,
          } satisfies DefaultsDraft,
        ]),
    ) as Partial<Record<ServiceKind, DefaultsDraft>>,
  );
  const updateDrafts = $state(
    Object.fromEntries(
      definitions.map((definition) => [
        definition.kind,
        {
          targetId: null,
          policy: 'inherit',
          start: 0,
          end: 0,
        } satisfies UpdateDraft,
      ]),
    ) as Record<ServiceKind, UpdateDraft>,
  );
  const transferReviews = $state(
    Object.fromEntries(
      definitions.map((definition) => [definition.kind, null]),
    ) as Record<ServiceKind, TransferReview | null>,
  );
  const releasedCompose = $state(
    Object.fromEntries(
      definitions.map((definition) => [definition.kind, false]),
    ) as Record<ServiceKind, boolean>,
  );
  const snapshots = $state<Partial<Record<ServiceKind, SupportSnapshot>>>({});
  const managerOptions = $state<Partial<Record<ServiceKind, ManagerOptions>>>(
    {},
  );
  const supportLimit = $state<Record<ServiceKind, number>>({
    radarr: 0,
    sonarr: 0,
    lidarr: 0,
    bazarr: 0,
    prowlarr: 0,
    nzbget: 0,
  });
  const subtitle = $state({
    language: 'en',
    forced: false,
    hearing: false,
  });

  let managers = $state<ManagerService[]>([]),
    support = $state<SupportService[]>([]),
    containers = $state<Container[]>([]),
    stackServices = $state<StackService[]>([]),
    provisions = $state<Provision[]>([]),
    policies = $state<UpdatePolicy[]>([]),
    updates = $state<Update[]>([]),
    updateTargets = $state<UpdateTarget[]>([]),
    approvalUsers = $state<ApprovalUser[]>([]),
    releases = $state<
      { kind: ServiceKind; image: string | null; tested_image: string }[]
    >([]);
  let timezone = $state(''),
    serverPolicy = $state<ServerUpdatePolicy>({
      policy: 'notify',
      window_start: 3,
      window_end: 5,
    }),
    busy = $state(false),
    message = $state('');
  const loadErrors = $state({
    managers: '',
    support: '',
    approval: '',
    updates: '',
    containers: '',
    stack: '',
  });

  function integration(definition: Definition) {
    return definition.role === 'manager'
      ? managers.find((service) => service.kind === definition.kind)
      : support.find((service) => service.kind === definition.kind);
  }
  function manager(kind: ServiceKind) {
    return managers.find((service) => service.kind === kind);
  }
  function supportService(kind: ServiceKind) {
    return support.find((service) => service.kind === kind);
  }
  function provision(kind: ServiceKind) {
    return provisions.find((item) => item.kind === kind);
  }
  function runtime(kind: ServiceKind) {
    return stackServices.find((item) => item.kind === kind);
  }
  function updateTarget(kind: ServiceKind) {
    return updateTargets.find((item) => item.kind === kind);
  }
  function updatePolicy(kind: ServiceKind) {
    const target = updateTarget(kind);
    return target
      ? policies.find((policy) => policy.service_id === target.id)
      : undefined;
  }
  function serviceUpdates(kind: ServiceKind) {
    const target = updateTarget(kind);
    return target
      ? updates.filter((update) => update.service_id === target.id)
      : [];
  }
  function managed(kind: ServiceKind) {
    const item = provision(kind);
    const service = definitions.find((definition) => definition.kind === kind);
    const connected = service ? integration(service) : undefined;
    return (
      item?.state === 'complete' &&
      !!item.service_id &&
      item.service_id === connected?.id
    );
  }
  function status(kind: ServiceKind) {
    const item = provision(kind);
    const live = runtime(kind);
    const definition = definitions.find((entry) => entry.kind === kind)!;
    const connected = integration(definition);
    if (item && item.state !== 'complete') return `Setup ${item.state}`;
    if (live)
      return live.drift
        ? 'Managed, configuration changed'
        : live.running
          ? 'Managed, running'
          : 'Managed, stopped';
    if (connected) return 'Connected';
    return 'Not connected';
  }
  function syncUpdateDrafts() {
    for (const definition of definitions) {
      const draft = updateDrafts[definition.kind];
      const target = updateTarget(definition.kind);
      if (!target) continue;
      if (draft.targetId === target.id) {
        if (draft.policy === 'inherit') {
          draft.start = serverPolicy.window_start;
          draft.end = serverPolicy.window_end;
        }
        continue;
      }
      const policy = updatePolicy(definition.kind);
      draft.policy = policy?.policy ?? 'inherit';
      const inherited = !policy || policy.policy === 'inherit';
      draft.start = inherited ? serverPolicy.window_start : policy.window_start;
      draft.end = inherited ? serverPolicy.window_end : policy.window_end;
      draft.targetId = target.id;
    }
  }
  async function loadUpdateData() {
    loadErrors.updates = '';
    try {
      const result = await api<{
        policies: UpdatePolicy[];
        timezone: string;
        items: Update[];
        services: UpdateTarget[];
        server_policy: ServerUpdatePolicy;
      }>('/admin/service-updates');
      policies = result.policies;
      updates = result.items;
      updateTargets = result.services;
      timezone = result.timezone;
      serverPolicy = result.server_policy;
      syncUpdateDrafts();
    } catch (error) {
      loadErrors.updates = String(error);
    }
  }
  async function loadManagers() {
    loadErrors.managers = '';
    try {
      managers = (await api<{ items: ManagerService[] }>('/admin/managers'))
        .items;
    } catch (error) {
      loadErrors.managers = String(error);
    }
  }
  async function loadSupport() {
    loadErrors.support = '';
    try {
      support = (await api<{ items: SupportService[] }>('/admin/support'))
        .items;
    } catch (error) {
      loadErrors.support = String(error);
    }
  }
  async function loadApprovalUsers() {
    loadErrors.approval = '';
    try {
      approvalUsers = (
        await api<{ items: ApprovalUser[] }>('/admin/acquisition/users')
      ).items;
    } catch (error) {
      loadErrors.approval = String(error);
    }
  }
  async function loadContainers() {
    if (!definitions.some((definition) => !integration(definition))) {
      containers = [];
      loadErrors.containers = '';
      return;
    }
    loadErrors.containers = '';
    try {
      containers = (
        await api<{ items: Container[] }>('/admin/managers/containers')
      ).items;
    } catch (error) {
      loadErrors.containers = String(error);
    }
  }
  async function loadStack() {
    loadErrors.stack = '';
    try {
      const stack = await api<{
        items: StackService[];
        provisions: Provision[];
      }>('/admin/stack');
      stackServices = stack.items;
      provisions = stack.provisions;
    } catch (error) {
      loadErrors.stack = String(error);
    }
  }
  async function refresh() {
    await Promise.all([
      loadManagers(),
      loadSupport(),
      loadApprovalUsers(),
      loadUpdateData(),
      loadStack(),
    ]);
    await loadContainers();
  }
  async function work(action: () => Promise<void>, success = '') {
    if (busy) return;
    busy = true;
    message = '';
    try {
      await action();
      if (success) message = success;
    } catch (error) {
      message = String(error);
    } finally {
      busy = false;
    }
  }
  async function connectExternal(definition: Definition) {
    const draft = setup[definition.kind];
    if (definition.role === 'manager') {
      await api('/admin/managers', 'POST', {
        name: draft.name,
        kind: definition.kind,
        container_id: draft.container,
        port: draft.port,
        api_key: draft.secret,
      });
    } else {
      await api('/admin/support', 'POST', {
        name: draft.name,
        kind: definition.kind,
        container_id: draft.container,
        port: draft.port,
        credentials: {
          username: definition.kind === 'nzbget' ? draft.username : '',
          secret: draft.secret,
        },
        native_url: draft.nativeUrl,
      });
    }
    draft.secret = '';
    await refresh();
  }
  async function installManaged(definition: Definition) {
    const draft = installs[definition.kind];
    await api('/admin/stack/install', 'POST', {
      kind: definition.kind,
      host_port: draft.hostPort,
      native_url: draft.nativeUrl,
    });
    await refresh();
  }
  async function loadManagerOptions(kind: ServiceKind) {
    const service = manager(kind);
    const draft = defaults[kind];
    if (!service || !draft) return;
    const options = await api<ManagerOptions>(
      `/admin/managers/${service.id}/options`,
    );
    managerOptions[kind] = options;
    draft.root_folder =
      service.defaults.root_folder ?? options.roots[0]?.path ?? '';
    draft.quality_profile =
      service.defaults.quality_profile ?? options.profiles[0]?.id ?? 0;
    draft.metadata_profile =
      service.defaults.metadata_profile ??
      options.metadata_profiles[0]?.id ??
      null;
    draft.monitored = service.defaults.monitored ?? true;
    draft.loaded = true;
  }
  async function saveManagerDefaults(kind: ServiceKind) {
    const service = manager(kind);
    const draft = defaults[kind];
    if (!service || !draft) return;
    await api(`/admin/managers/${service.id}/defaults`, 'PUT', {
      root_folder: draft.root_folder,
      quality_profile: draft.quality_profile,
      metadata_profile: draft.metadata_profile,
      monitored: draft.monitored,
    });
    await refresh();
  }
  async function testManager(kind: ServiceKind) {
    const service = manager(kind);
    if (!service) return;
    await api(`/admin/managers/${service.id}/test`, 'POST');
    await refresh();
  }
  async function refreshSupport(kind: ServiceKind) {
    const service = supportService(kind);
    if (!service) return;
    const snapshot = await api<SupportSnapshot>(`/admin/support/${service.id}`);
    snapshots[kind] = snapshot;
    if (kind === 'nzbget') supportLimit.nzbget = snapshot.limit ?? 0;
  }
  async function supportCommand(
    kind: ServiceKind,
    action: string,
    extra: Record<string, unknown> = {},
  ) {
    const service = supportService(kind);
    if (!service) return;
    await api(`/admin/support/${service.id}`, 'POST', { action, ...extra });
    await refreshSupport(kind);
  }
  async function previewAdoption(definition: Definition) {
    const connected = integration(definition);
    if (!connected) return;
    transferReviews[definition.kind] = await api<TransferReview>(
      '/admin/stack/adopt/preview',
      'POST',
      { service_id: connected.id },
    );
    releasedCompose[definition.kind] = false;
  }
  async function adopt(definition: Definition) {
    const connected = integration(definition);
    const review = transferReviews[definition.kind];
    if (!connected || !review) return;
    await api('/admin/stack/adopt', 'POST', {
      service_id: connected.id,
      review_id: review.review_id,
      released_compose: releasedCompose[definition.kind],
    });
    transferReviews[definition.kind] = null;
    await refresh();
  }
  async function stackAction(kind: ServiceKind, action: string) {
    const service = runtime(kind);
    if (!service) return;
    await api(`/admin/stack/${service.id}/action`, 'POST', { action });
    await refresh();
  }
  async function retryProvision(kind: ServiceKind) {
    const item = provision(kind);
    if (!item) return;
    await api(`/admin/stack/${item.id}/retry`, 'POST', {});
    await refresh();
  }
  async function restoreOriginal(kind: ServiceKind) {
    const item = provision(kind);
    if (!item) return;
    await api(`/admin/stack/${item.id}/restore-original`, 'POST', {});
    await refresh();
  }
  async function saveServiceUpdatePolicy(kind: ServiceKind) {
    const target = updateTarget(kind);
    if (!target) return;
    const draft = updateDrafts[kind];
    await api(`/admin/service-updates/policy/${target.id}`, 'POST', {
      policy: draft.policy,
      window_start: draft.start,
      window_end: draft.end,
    });
    await loadUpdateData();
  }
  async function preflight(kind: ServiceKind) {
    const target = updateTarget(kind);
    if (!target) return;
    await api(`/admin/service-updates/preflight/${target.id}`, 'POST', {});
    await loadUpdateData();
  }
  async function updateAction(id: string, action: 'activate' | 'recover') {
    await api(`/admin/service-updates/${id}/${action}`, 'POST', {});
    await loadUpdateData();
  }
  async function discoverReleases() {
    releases = (await api<{ items: typeof releases }>('/admin/stack/releases'))
      .items;
  }

  onMount(() => {
    void work(refresh);
    const timer = setInterval(() => {
      if (busy) return;
      if (
        provisions.some((item) =>
          ['queued', 'installing', 'connecting'].includes(item.state),
        )
      )
        void refresh().catch(() => {});
      else void loadUpdateData().catch(() => {});
    }, 4000);
    return () => clearInterval(timer);
  });
</script>

<Panel aria-label="Media services" class="grid gap-4">
  <div class={sectionHeadingClass}>
    <h2>Media services</h2>
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void work(refresh)}>Refresh</Button
    >
  </div>
  {#if message}<p role="status">{message}</p>{/if}
  {#each loaderSubsystems as subsystem (subsystem.key)}
    {#if loadErrors[subsystem.key]}
      <div
        class="mb-3 flex items-start gap-3 border border-danger bg-danger/12 px-4 py-3 text-danger"
        role="alert"
        aria-label={`${subsystem.label} load error`}
      >
        <CircleX class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div class="min-w-0">
          <strong class="block text-xs font-semibold"
            >Failed to load {subsystem.label}</strong
          >
          <p class="mt-1 mb-0 text-xs leading-5 wrap-anywhere">
            {loadErrors[subsystem.key]}
          </p>
        </div>
      </div>
    {/if}
  {/each}

  {#if approvalUsers.length}
    <section class="border border-line bg-surface-soft p-4">
      <h3 class="mb-3 text-sm">Automatic request approval</h3>
      <div class="flex flex-wrap gap-3">
        {#each approvalUsers as user (user.id)}
          <Switch
            size="sm"
            checked={user.enabled}
            disabled={busy}
            onCheckedChange={(enabled) =>
              void work(async () => {
                await api(`/admin/acquisition/users/${user.id}`, 'PUT', {
                  enabled,
                });
                await refresh();
              })}
          >
            {user.username}</Switch
          >
        {/each}
      </div>
    </section>
  {/if}

  <div class="flex flex-wrap gap-2">
    <Button
      variant="secondary"
      size="form"
      disabled={busy || Boolean(loadErrors.stack)}
      onclick={() =>
        void work(async () => {
          await api('/admin/stack/wire', 'POST', {});
          await refresh();
        }, 'Installed services connected.')}>Connect installed services</Button
    >
    <Button
      variant="secondary"
      size="form"
      disabled={busy || Boolean(loadErrors.stack)}
      onclick={() => void work(discoverReleases)}
      >Discover stable releases</Button
    >
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() =>
        void work(async () => {
          await api('/admin/service-updates/check', 'POST', {});
          await loadUpdateData();
        })}>Check stable updates</Button
    >
  </div>
  {#if releases.length}
    <div class="grid gap-1 text-xs">
      {#each releases as release (release.kind)}
        <p class="m-0 wrap-anywhere">
          <strong class="capitalize">{release.kind}</strong>:
          {release.image === release.tested_image
            ? 'matches the tested template'
            : release.image
              ? 'needs compatibility verification'
              : 'could not be verified'}{release.image
            ? ` · ${release.image}`
            : ''}
        </p>
      {/each}
    </div>
  {/if}

  <div class="grid gap-2">
    {#each definitions as definition (definition.kind)}
      {@const connected = integration(definition)}
      {@const item = provision(definition.kind)}
      {@const live = runtime(definition.kind)}
      {@const target = updateTarget(definition.kind)}
      {@const policy = updatePolicy(definition.kind)}
      {@const servicePolicy = updateDrafts[definition.kind]}
      {@const serviceUpdatesList = serviceUpdates(definition.kind)}
      {@const supportData = snapshots[definition.kind]}
      {@const provisioning =
        !!item && ['queued', 'installing', 'connecting'].includes(item.state)}
      <article
        aria-label={`${definition.label} service`}
        class="min-w-0 border border-line bg-surface-soft"
      >
        <details>
          <summary
            class="flex cursor-pointer list-none items-center justify-between gap-4 p-4 [&::-webkit-details-marker]:hidden"
          >
            <span class="min-w-0">
              <strong class="text-[13px] font-[600]">{definition.label}</strong>
              {#if connected}<span class="ml-2 text-[11px] text-muted"
                  >{connected.version}</span
                >{/if}
            </span>
            <span class={badgeClass}>{status(definition.kind)}</span>
          </summary>
          <div
            class="service-body border-t border-line p-4 [&>details]:border-t [&>details]:border-line [&>details]:py-3 [&>details>summary]:cursor-pointer [&>details>summary]:text-xs [&>details>summary]:font-semibold [&>details>summary]:tracking-[0.04em] [&_button]:mr-2 [&_button]:mb-2 [&_p]:my-2 [&_p]:text-xs [&_p]:leading-5"
          >
            {#if connected}
              <p class="wrap-anywhere">
                <strong>{connected.name}</strong> · {connected.version}
                {#if managed(definition.kind)}
                  · owned by Thelxinoe{/if}
              </p>
              {#if connected.error}<p role="status">{connected.error}</p>{/if}
            {:else if item}
              <p>
                Managed setup is {item.state}.{item.error
                  ? ` ${item.error}`
                  : ''}
              </p>
            {/if}

            {#if !connected && !item && !live}
              <details>
                <summary>Connect {definition.label}</summary>
                <div class="mt-3 grid gap-4">
                  <form
                    class="grid gap-3 [&_label]:m-0"
                    aria-label={`Install managed ${definition.label}`}
                    onsubmit={(event) => {
                      event.preventDefault();
                      void work(
                        () => installManaged(definition),
                        `${definition.label} installation queued.`,
                      );
                    }}
                  >
                    <strong class="text-xs">Install and own it</strong>
                    <div class="grid grid-cols-2 gap-3 compact:grid-cols-1">
                      <label
                        >Local port<input
                          type="number"
                          min="1024"
                          max="65535"
                          bind:value={installs[definition.kind].hostPort}
                          required
                        /></label
                      >
                      <label
                        >Advanced UI address<input
                          type="url"
                          bind:value={installs[definition.kind].nativeUrl}
                          placeholder="Optional"
                        /></label
                      >
                    </div>
                    <Button
                      type="submit"
                      variant="secondary"
                      size="form"
                      disabled={busy || Boolean(loadErrors.stack)}
                      >Install {definition.label}</Button
                    >
                  </form>
                  <form
                    class="grid gap-3 border-t border-line pt-3 [&_label]:m-0"
                    aria-label={`Connect existing ${definition.label}`}
                    onsubmit={(event) => {
                      event.preventDefault();
                      void work(
                        () => connectExternal(definition),
                        `${definition.label} connected.`,
                      );
                    }}
                  >
                    <strong class="text-xs"
                      >Connect an existing container</strong
                    >
                    <label
                      >Name<input
                        bind:value={setup[definition.kind].name}
                        required
                        maxlength="100"
                      /></label
                    >
                    <div class="grid grid-cols-2 gap-3 compact:grid-cols-1">
                      <label
                        >Container<select
                          bind:value={setup[definition.kind].container}
                          required
                          ><option value="">Select container</option
                          >{#each containers as container (container.id)}
                            <option value={container.id}
                              >{container.names[0]}{container.state
                                ? ` (${container.state})`
                                : ''}</option
                            >
                          {/each}</select
                        ></label
                      >
                      <label
                        >Internal port<input
                          type="number"
                          min="1"
                          max="65535"
                          bind:value={setup[definition.kind].port}
                          required
                        /></label
                      >
                    </div>
                    {#if definition.kind === 'nzbget'}
                      <label
                        >NZBGet username<input
                          bind:value={setup.nzbget.username}
                          required
                          autocomplete="off"
                        /></label
                      >
                    {/if}
                    <label
                      >{definition.kind === 'nzbget'
                        ? 'NZBGet password'
                        : 'API key'}<input
                        type="password"
                        bind:value={setup[definition.kind].secret}
                        required
                        autocomplete="new-password"
                      /></label
                    >
                    {#if definition.role === 'support'}
                      <label
                        >Service UI address<input
                          type="url"
                          bind:value={setup[definition.kind].nativeUrl}
                          placeholder="https://service.example.com"
                        /></label
                      >
                    {/if}
                    <Button
                      type="submit"
                      size="form"
                      disabled={busy || !containers.length}
                      >Connect {definition.label}</Button
                    >
                  </form>
                </div>
              </details>
            {/if}

            {#if connected}
              <details>
                <summary>{definition.label} settings</summary>
                <div class="mt-3">
                  {#if definition.role === 'manager'}
                    <Button
                      variant="secondary"
                      size="form"
                      disabled={busy}
                      onclick={() =>
                        void work(
                          () => testManager(definition.kind),
                          'Connection and mount checks passed.',
                        )}>Test connection</Button
                    >
                    <Button
                      variant="secondary"
                      size="form"
                      disabled={busy}
                      onclick={() =>
                        void work(() => loadManagerOptions(definition.kind))}
                      >{defaults[definition.kind]?.loaded
                        ? 'Reload profile choices'
                        : 'Load acquisition defaults'}</Button
                    >
                    {#if defaults[definition.kind]?.loaded}
                      {@const options = managerOptions[definition.kind]}
                      {@const draft = defaults[definition.kind]!}
                      {#if options}
                        <form
                          class="mt-3 grid gap-3 [&_label]:m-0"
                          aria-label={`${definition.label} acquisition defaults`}
                          onsubmit={(event) => {
                            event.preventDefault();
                            void work(
                              () => saveManagerDefaults(definition.kind),
                              'Acquisition defaults saved.',
                            );
                          }}
                        >
                          {#if !options.roots.length}<p>
                              Add a root folder in {definition.label} first.
                            </p>{/if}
                          {#if options.roots[0]?.id === 0}<p>
                              The canonical library folder will be created when
                              these defaults are saved.
                            </p>{/if}
                          <label
                            >Root folder<input
                              value={draft.root_folder}
                              readonly
                            /></label
                          >
                          <label
                            >Quality profile<select
                              bind:value={draft.quality_profile}
                              required
                              >{#each options.profiles as option (option.id)}
                                <option value={option.id}>{option.name}</option>
                              {/each}</select
                            ></label
                          >
                          {#if definition.kind === 'lidarr'}
                            <label
                              >Metadata profile<select
                                bind:value={draft.metadata_profile}
                                required
                                >{#each options.metadata_profiles as option (option.id)}
                                  <option value={option.id}
                                    >{option.name}</option
                                  >
                                {/each}</select
                              ></label
                            >
                          {/if}
                          <Switch bind:checked={draft.monitored} size="sm"
                            >Monitor and search approved requests</Switch
                          >
                          <Button
                            type="submit"
                            size="form"
                            disabled={busy ||
                              !draft.root_folder ||
                              !draft.quality_profile}>Save defaults</Button
                          >
                        </form>
                      {/if}
                    {/if}
                  {:else}
                    {@const service = supportService(definition.kind)}
                    {#if service?.native_url}
                      <a
                        class="mr-3 text-xs"
                        href={service.native_url}
                        target="_blank"
                        rel="noopener noreferrer">Open {definition.label}</a
                      >
                    {/if}
                    <Button
                      variant="secondary"
                      size="form"
                      disabled={busy}
                      onclick={() =>
                        void work(() => refreshSupport(definition.kind))}
                      >Refresh status</Button
                    >
                    {#if supportData}
                      {#each supportData.health ?? [] as issue, index (index)}
                        <p>
                          {typeof issue === 'string' ? issue : issue.message}
                        </p>
                      {/each}
                      {#if definition.kind === 'prowlarr'}
                        {#if !supportData.indexers?.length}<p>
                            No indexers configured. Open Prowlarr to add one.
                          </p>{/if}
                        {#each supportData.indexers ?? [] as indexer (indexer.id)}
                          <div class="border-t border-line py-2">
                            <p>
                              {indexer.name} · {indexer.enabled
                                ? 'Enabled'
                                : 'Disabled'}{indexer.disabled_until
                                ? ` · unavailable until ${indexer.disabled_until}`
                                : ''}
                            </p>
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={busy}
                              onclick={() =>
                                void work(() =>
                                  supportCommand('prowlarr', 'test', {
                                    item_id: indexer.id,
                                  }),
                                )}>Test</Button
                            >
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={busy}
                              onclick={() =>
                                void work(() =>
                                  supportCommand(
                                    'prowlarr',
                                    indexer.enabled ? 'disable' : 'enable',
                                    { item_id: indexer.id },
                                  ),
                                )}
                              >{indexer.enabled ? 'Disable' : 'Enable'}</Button
                            >
                          </div>
                        {/each}
                      {:else if definition.kind === 'nzbget'}
                        <p>
                          {supportData.paused
                            ? 'Downloads paused'
                            : 'Downloads running'} · {(
                            (supportData.rate ?? 0) / 1024
                          ).toFixed(0)} KB/s · {(
                            (supportData.free_mb ?? 0) / 1024
                          ).toFixed(1)} GB free
                        </p>
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={busy}
                          onclick={() =>
                            void work(() =>
                              supportCommand(
                                'nzbget',
                                supportData.paused ? 'resume_all' : 'pause_all',
                              ),
                            )}
                          >{supportData.paused
                            ? 'Resume all'
                            : 'Pause all'}</Button
                        >
                        <form
                          class="my-3 flex items-end gap-2 compact:flex-col compact:items-stretch [&_label]:m-0"
                          onsubmit={(event) => {
                            event.preventDefault();
                            void work(() =>
                              supportCommand('nzbget', 'rate', {
                                value: supportLimit.nzbget,
                              }),
                            );
                          }}
                        >
                          <label
                            >Download limit (KB/s)<input
                              type="number"
                              min="0"
                              max="1000000"
                              bind:value={supportLimit.nzbget}
                            /></label
                          >
                          <Button type="submit" variant="secondary" size="form"
                            >Set limit</Button
                          >
                        </form>
                        {#each supportData.queue ?? [] as download (download.id)}
                          <div class="border-t border-line py-2">
                            <p>
                              {download.title} · {download.status} ·
                              {download.remaining_mb}/{download.size_mb} MB remaining
                            </p>
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={busy}
                              onclick={() =>
                                void work(() =>
                                  supportCommand('nzbget', 'pause', {
                                    item_id: download.id,
                                  }),
                                )}>Pause</Button
                            >
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={busy}
                              onclick={() =>
                                void work(() =>
                                  supportCommand('nzbget', 'resume', {
                                    item_id: download.id,
                                  }),
                                )}>Resume</Button
                            >
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={busy}
                              onclick={() =>
                                void work(() =>
                                  supportCommand('nzbget', 'remove', {
                                    item_id: download.id,
                                  }),
                                )}>Remove</Button
                            >
                          </div>
                        {/each}
                        {#if supportData.history?.length}
                          <h4 class="mt-3 text-xs">Recent downloads</h4>
                          {#each supportData.history as download (download.id)}
                            <p>{download.title} · {download.status}</p>
                          {/each}
                        {/if}
                      {:else}
                        <div class="my-3 grid gap-3 [&_label]:m-0">
                          <label
                            >Subtitle language<input
                              bind:value={subtitle.language}
                              minlength="2"
                              maxlength="3"
                              placeholder="en"
                            /></label
                          >
                          <div class="flex flex-wrap gap-3">
                            <Switch bind:checked={subtitle.forced} size="sm"
                              >Forced subtitles</Switch
                            >
                            <Switch bind:checked={subtitle.hearing} size="sm"
                              >Hearing impaired</Switch
                            >
                          </div>
                        </div>
                        {#if !supportData.movies?.length && !supportData.episodes?.length}<p
                          >
                            No missing subtitles reported. Configure language
                            profiles and providers in Bazarr.
                          </p>{/if}
                        {#each [...(supportData.movies ?? []), ...(supportData.episodes ?? [])] as missing (`${missing.movie_id}:${missing.episode_id}`)}
                          <div class="border-t border-line py-2">
                            <p>{missing.title}</p>
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={busy}
                              onclick={() =>
                                void work(() =>
                                  supportCommand('bazarr', 'subtitles', {
                                    item_id:
                                      missing.movie_id ?? missing.episode_id,
                                    domain: missing.movie_id
                                      ? 'movies'
                                      : 'episodes',
                                    language: subtitle.language,
                                    forced: subtitle.forced,
                                    hearing_impaired: subtitle.hearing,
                                  }),
                                )}>Find subtitles</Button
                            >
                          </div>
                        {/each}
                      {/if}
                    {/if}
                  {/if}
                </div>
              </details>
            {/if}

            {#if connected && !item && !live}
              <details>
                <summary>Ownership</summary>
                <div class="mt-3">
                  <p>
                    Thelxinoe can take ownership of this container so service
                    lifecycle and updates can be managed here.
                  </p>
                  <Button
                    variant="secondary"
                    size="form"
                    disabled={busy || Boolean(loadErrors.stack)}
                    onclick={() => void work(() => previewAdoption(definition))}
                    >Review ownership transfer</Button
                  >
                  {#if transferReviews[definition.kind]}
                    {@const review = transferReviews[definition.kind]!}
                    <div class="mt-3 border-t border-line pt-3">
                      <p>
                        Copy configuration from
                        <code>{review.source_config}</code> to
                        <code>{review.managed_config}</code>.
                      </p>
                      <details>
                        <summary>Version retained during transfer</summary>
                        <code class="text-xs wrap-anywhere">{review.image}</code
                        >
                      </details>
                      {#if review.compose_project}
                        <p>
                          Disable <strong>{review.compose_service}</strong> in
                          Compose project
                          <strong>{review.compose_project}</strong> before the transfer.
                        </p>
                        <Switch
                          size="sm"
                          bind:checked={releasedCompose[definition.kind]}
                          >I disabled the previous Compose service.</Switch
                        >
                      {:else}
                        <p>
                          Disable any script or external updater that recreates
                          this container before transferring it.
                        </p>
                      {/if}
                      <div class="mt-3">
                        <Button
                          variant="secondary"
                          size="form"
                          disabled={busy ||
                            (!!review.compose_project &&
                              !releasedCompose[definition.kind])}
                          onclick={() =>
                            void work(
                              () => adopt(definition),
                              `${definition.label} ownership transfer queued.`,
                            )}>Stop, copy and take ownership</Button
                        >
                        <Button
                          variant="ghost"
                          size="form"
                          disabled={busy}
                          onclick={() =>
                            (transferReviews[definition.kind] = null)}
                          >Cancel</Button
                        >
                      </div>
                    </div>
                  {/if}
                </div>
              </details>
            {/if}

            {#if item || live}
              <details>
                <summary>Managed service</summary>
                <div class="mt-3">
                  {#if item}
                    <p>
                      Provision: {item.state}{item.host_port
                        ? ` · port ${item.host_port}`
                        : ''}{item.error ? ` · ${item.error}` : ''}
                    </p>
                    {#if item.native_url}<a
                        class="text-xs"
                        href={item.native_url}
                        target="_blank"
                        rel="noreferrer">Open service UI</a
                      >{/if}
                    {#if item.state === 'blocked'}
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={busy}
                        onclick={() =>
                          void work(() => retryProvision(definition.kind))}
                        >Retry setup</Button
                      >
                      {#if item.origin === 'adopted' && !stackServices.some((service) => service.id === item.id && !service.transfer_pending)}
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={busy}
                          onclick={() =>
                            void work(() => restoreOriginal(definition.kind))}
                          >Restore original</Button
                        >
                      {/if}
                    {/if}
                  {/if}
                  {#if live}
                    <p>
                      {live.drift
                        ? 'Configuration changed outside Thelxinoe'
                        : live.running
                          ? 'Running'
                          : 'Stopped'} · {live.phase}
                    </p>
                    {#if live.error}<p>{live.error}</p>{/if}
                    <details>
                      <summary>Installed image</summary>
                      <code class="text-xs wrap-anywhere">{live.image}</code>
                    </details>
                    <div class="mt-3">
                      {#each ['start', 'stop', 'restart', 'reconcile'] as action (action)}
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={busy ||
                            provisioning ||
                            (action !== 'reconcile' &&
                              (live.drift || live.phase !== 'active'))}
                          onclick={() =>
                            void work(() =>
                              stackAction(definition.kind, action),
                            )}>{action}</Button
                        >
                      {/each}
                    </div>
                  {/if}
                </div>
              </details>
            {/if}

            <details>
              <summary>Updates</summary>
              <div class="mt-3">
                {#if target}
                  <form
                    class="grid gap-3 [&_label]:m-0"
                    aria-label={`${definition.label} update settings`}
                    onsubmit={(event) => {
                      event.preventDefault();
                      void work(
                        () => saveServiceUpdatePolicy(definition.kind),
                        `${definition.label} update policy saved.`,
                      );
                    }}
                  >
                    <label
                      >Update policy<select bind:value={servicePolicy.policy}
                        ><option value="inherit"
                          >Use server update policy</option
                        ><option value="notify">Notify</option><option
                          value="automatic">Automatic</option
                        ><option value="manual">Manual</option></select
                      ></label
                    >
                    {#if servicePolicy.policy !== 'inherit'}
                      <div class="grid grid-cols-2 gap-3 compact:grid-cols-1">
                        <label
                          >Maintenance starts ({timezone})<input
                            type="number"
                            min="0"
                            max="23"
                            bind:value={servicePolicy.start}
                            required
                          /></label
                        >
                        <label
                          >Maintenance ends ({timezone})<input
                            type="number"
                            min="0"
                            max="23"
                            bind:value={servicePolicy.end}
                            required
                          /></label
                        >
                      </div>
                    {:else}
                      <p class="text-muted">
                        Uses the Server update policy ({serverPolicy.policy},
                        {String(serverPolicy.window_start).padStart(
                          2,
                          '0',
                        )}:00–{String(serverPolicy.window_end).padStart(
                          2,
                          '0',
                        )}:00).
                      </p>
                    {/if}
                    <Button type="submit" size="form" disabled={busy}
                      >Save update policy</Button
                    >
                  </form>
                  {#if policy?.candidate}<p class="wrap-anywhere">
                      Stable candidate: {policy.candidate}
                    </p>{/if}
                  {#if policy?.error}<p role="status">{policy.error}</p>{/if}
                  <Button
                    variant="secondary"
                    size="form"
                    disabled={busy}
                    onclick={() => void work(() => preflight(definition.kind))}
                    >Check compatibility</Button
                  >
                  {#each serviceUpdatesList as update (update.id)}
                    <div class="mt-2 border-t border-line pt-2">
                      <strong class="text-xs"
                        >{stages[update.state] ?? update.state}</strong
                      >
                      {#if update.classification === 'incompatible'}<p>
                          Candidate did not pass compatibility checks.
                        </p>{:else if update.classification === 'unable-to-verify'}<p
                        >
                          Compatibility could not be verified.
                        </p>{/if}
                      {#if update.error}<p>{update.error}</p>{/if}
                      {#if update.state === 'ready'}
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={busy}
                          onclick={() =>
                            void work(() =>
                              updateAction(update.id, 'activate'),
                            )}>Install verified update</Button
                        >
                      {/if}
                      {#if ['blocked', 'recovery-required'].includes(update.state)}
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={busy}
                          onclick={() =>
                            void work(() => updateAction(update.id, 'recover'))}
                          >Recover before activation</Button
                        >
                      {/if}
                      {#if update.state === 'runtime-failure'}<p>
                          The updated service needs attention before any
                          explicit restore.
                        </p>{/if}
                    </div>
                  {/each}
                {:else}
                  <p class="text-muted">
                    Update settings become available after Thelxinoe owns this
                    service.
                  </p>
                {/if}
              </div>
            </details>
          </div>
        </details>
      </article>
    {/each}
  </div>
</Panel>
