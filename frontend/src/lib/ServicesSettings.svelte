<script lang="ts">
  import { onMount } from 'svelte';
  import { api, serverUrl } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import Switch from './ui/Switch.svelte';
  import { ExternalLink, RefreshCw } from '@lucide/svelte';
  import DownloadsTable from './services/DownloadsTable.svelte';
  import type { SupportDownload } from './services/downloads';
  import { serviceUiUrl, type Container } from './services/presentation';
  import radarrIcon from './services/icons/radarr.png';
  import sonarrIcon from './services/icons/sonarr.png';
  import lidarrIcon from './services/icons/lidarr.png';
  import bazarrIcon from './services/icons/bazarr.png';
  import prowlarrIcon from './services/icons/prowlarr.png';
  import nzbgetIcon from './services/icons/nzbget.png';
  import ServiceConnections, {
    type ServiceConnection,
  } from './ServiceConnections.svelte';

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
    container_id: string;
    port: number;
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
    drift: boolean | null;
    running: boolean | null;
    existence: 'present' | 'missing' | 'unknown';
    status: 'running' | 'stopped' | 'missing' | 'unavailable' | 'drifted';
    inspection_error: string | null;
    can_recreate: boolean;
    can_retire: boolean;
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
      description: 'Movie manager',
    },
    {
      kind: 'sonarr',
      label: 'Sonarr',
      role: 'manager',
      internalPort: 8989,
      hostPort: 18989,
      description: 'Series manager',
    },
    {
      kind: 'lidarr',
      label: 'Lidarr',
      role: 'manager',
      internalPort: 8686,
      hostPort: 18686,
      description: 'Music manager',
    },
    {
      kind: 'bazarr',
      label: 'Bazarr',
      role: 'support',
      internalPort: 6767,
      hostPort: 16767,
      description: 'Subtitles',
    },
    {
      kind: 'prowlarr',
      label: 'Prowlarr',
      role: 'support',
      internalPort: 9696,
      hostPort: 19696,
      description: 'Indexers',
    },
    {
      kind: 'nzbget',
      label: 'NZBGet',
      role: 'support',
      internalPort: 6789,
      hostPort: 16789,
      description: 'Usenet downloads',
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
    committed: 'Update complete',
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
    { key: 'connections', label: 'Optional service connections' },
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
  const subtitle = $state({
    language: 'en',
    forced: false,
    hearing: false,
  });

  let managers = $state<ManagerService[]>([]),
    connections = $state<ServiceConnection[]>([]),
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
  let nzbgetLogin = $state<{
    id: string;
    username: string;
    password: string;
  } | null>(null);
  let showNzbgetPassword = $state(false);
  const loadErrors = $state({
    managers: '',
    support: '',
    approval: '',
    updates: '',
    containers: '',
    stack: '',
    connections: '',
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
  const icons = {
    radarr: radarrIcon,
    sonarr: sonarrIcon,
    lidarr: lidarrIcon,
    bazarr: bazarrIcon,
    prowlarr: prowlarrIcon,
    nzbget: nzbgetIcon,
  };
  let selectedKind = $state<ServiceKind>('radarr');
  let busyKind = $state<ServiceKind | null>(null);
  const detailErrors = $state<Partial<Record<ServiceKind, string>>>({});
  const detailLoading = $state<Partial<Record<ServiceKind, boolean>>>({});
  const definition = $derived(
    definitions.find((entry) => entry.kind === selectedKind)!,
  );
  const connected = $derived(integration(definition));
  const item = $derived(provision(selectedKind));
  const live = $derived(runtime(selectedKind));
  const supportData = $derived(snapshots[selectedKind]);
  const nzbgetLoginTarget = $derived(
    provision('nzbget') ?? supportService('nzbget'),
  );
  const target = $derived(updateTarget(selectedKind));
  const policy = $derived(updatePolicy(selectedKind));
  const servicePolicy = $derived(updateDrafts[selectedKind]);
  const serviceUpdatesList = $derived(serviceUpdates(selectedKind).slice(0, 1));

  function setupActive(kind: ServiceKind) {
    return ['queued', 'installing', 'connecting', 'retiring'].includes(
      provision(kind)?.state ?? '',
    );
  }
  function activity(kind: ServiceKind) {
    const state = provision(kind)?.state;
    if (setupActive(kind))
      return (
        {
          queued: 'Waiting to install',
          installing: 'Installing',
          connecting: 'Connecting API',
          retiring: 'Retiring installation',
        } as Record<string, string>
      )[state!];
    const update = serviceUpdates(kind)
      .slice(0, 1)
      .find((entry) =>
        [
          'queued',
          'submitting',
          'preparing',
          'snapshotting',
          'preflight',
          'queued-activate',
          'recovery-snapshot',
          'isolated-live-validation',
          'activating',
          'queued-recover',
        ].includes(entry.state),
      );
    if (update)
      return update.state === 'activating'
        ? 'Connecting API'
        : stages[update.state];
    return busy && busyKind === kind ? 'Working' : '';
  }
  function status(kind: ServiceKind) {
    const item = provision(kind),
      live = runtime(kind);
    const connected = integration(
      definitions.find((entry) => entry.kind === kind)!,
    );
    if (setupActive(kind))
      return {
        label:
          item?.state === 'connecting'
            ? 'Connecting API'
            : item?.state === 'retiring'
              ? 'Retiring'
              : 'Installing',
        tone: 'busy',
      };
    if (item?.state === 'blocked')
      return { label: 'Setup blocked', tone: 'bad' };
    if (serviceUpdates(kind)[0]?.state === 'activating')
      return { label: 'Connecting API', tone: 'busy' };
    if (live?.status === 'unavailable')
      return { label: 'Status unavailable', tone: 'warn' };
    if (live?.status === 'missing')
      return { label: 'Container missing', tone: 'bad' };
    if (live?.running === false) return { label: 'Stopped', tone: 'muted' };
    if (connected?.error) return { label: 'API unavailable', tone: 'warn' };
    if (live?.running) return { label: 'Running', tone: 'ok' };
    if (connected) return { label: 'Connected', tone: 'ok' };
    if (
      loadErrors[
        definitions.find((entry) => entry.kind === kind)!.role === 'manager'
          ? 'managers'
          : 'support'
      ] ||
      loadErrors.stack
    )
      return { label: 'Status unavailable', tone: 'warn' };
    return { label: 'Not connected', tone: 'muted' };
  }
  function containerFor(definition: Definition) {
    const id =
      provision(definition.kind)?.container_id ??
      integration(definition)?.container_id;
    return containers.find((container) => container.id === id);
  }
  function externalUrl(definition: Definition) {
    const connected = integration(definition);
    return serviceUiUrl(
      provision(definition.kind)?.native_url ||
        supportService(definition.kind)?.native_url,
      containerFor(definition)?.ports ?? [],
      connected?.port ?? definition.internalPort,
      serverUrl() || window.location.origin,
    );
  }
  function canControl(kind: ServiceKind, action: string) {
    const live = runtime(kind);
    return (
      !busy &&
      !setupActive(kind) &&
      !!live &&
      live.existence !== 'unknown' &&
      (action === 'reconcile' ||
        (live.existence === 'present' &&
          !live.drift &&
          live.phase === 'active'))
    );
  }
  async function loadSelectedData() {
    const kind = selectedKind;
    if (detailLoading[kind]) return;
    detailLoading[kind] = true;
    detailErrors[kind] = '';
    try {
      if (manager(kind) && !defaults[kind]?.loaded)
        await loadManagerOptions(kind);
      else if (supportService(kind)) await refreshSupport(kind);
    } catch (error) {
      detailErrors[kind] = String(error);
    } finally {
      detailLoading[kind] = false;
    }
  }
  function selectService(kind: ServiceKind) {
    nzbgetLogin = null;
    showNzbgetPassword = false;
    selectedKind = kind;
    void loadSelectedData();
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
      stackServices = stackServices.map((service) => ({
        ...service,
        status: 'unavailable',
        existence: 'unknown',
        running: null,
        drift: null,
        can_recreate: false,
        can_retire: false,
      }));
    }
  }
  async function refresh() {
    await Promise.all([
      loadManagers(),
      loadSupport(),
      loadApprovalUsers(),
      loadUpdateData(),
      loadStack(),
      loadConnections(),
    ]);
    await loadContainers();
    await loadSelectedData();
  }
  async function loadConnections() {
    loadErrors.connections = '';
    try {
      connections = (
        await api<{ items: ServiceConnection[] }>('/admin/service-connections')
      ).items;
    } catch (error) {
      loadErrors.connections = String(error);
    }
  }
  async function connectionAction(
    connection: ServiceConnection,
    action: 'connect' | 'disconnect' | 'retry',
  ) {
    await api('/admin/service-connections', 'POST', {
      source_id: connection.source_id,
      target_id: connection.target_id,
      action,
    });
    await loadConnections();
  }
  async function work(action: () => Promise<void>, success = '') {
    if (busy) return;
    busy = true;
    busyKind = selectedKind;
    message = '';
    try {
      await action();
      if (success) message = success;
    } catch (error) {
      message = String(error);
    } finally {
      busy = false;
      busyKind = null;
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
  }
  async function revealNzbgetLogin() {
    const service = nzbgetLoginTarget;
    if (!service) return;
    const source = provision('nzbget') ? 'stack' : 'support';
    const login = await api<{ username: string; password: string }>(
      `/admin/${source}/${service.id}/login`,
      'POST',
    );
    if (selectedKind === 'nzbget' && nzbgetLoginTarget?.id === service.id)
      nzbgetLogin = { id: service.id, ...login };
  }
  async function copyNzbgetLogin(value: string, label: string) {
    if (!navigator.clipboard?.writeText)
      throw new Error('Clipboard unavailable. Select the value to copy it.');
    await navigator.clipboard.writeText(value);
    message = `${label} copied to clipboard.`;
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
    const service = runtime(kind) ?? provision(kind);
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
    let polling = false;
    const timer = setInterval(async () => {
      if (busy || polling || document.hidden) return;
      polling = true;
      try {
        if (
          provisions.some((item) =>
            ['queued', 'installing', 'connecting'].includes(item.state),
          )
        )
          await refresh();
        else
          await Promise.all([
            loadStack(),
            loadUpdateData(),
            loadConnections(),
            loadManagers(),
            loadSupport(),
          ]);
        await loadSelectedData();
      } finally {
        polling = false;
      }
    }, 4000);
    return () => clearInterval(timer);
  });
</script>

<Panel aria-label="Media services" class="services-panel">
  <nav class="service-strip" aria-label="Select service">
    {#each definitions as service (service.kind)}
      {@const state = status(service.kind)}
      <button
        class="service-tab"
        aria-current={selectedKind === service.kind ? 'true' : undefined}
        aria-label={service.label}
        onclick={() => selectService(service.kind)}
      >
        <img src={icons[service.kind]} alt="" />
        <span
          ><strong>{service.label}</strong><span
            class="service-status"
            data-tone={state.tone}>{state.label}</span
          ></span
        >
      </button>
    {/each}
  </nav>
  {#each loaderSubsystems as subsystem (subsystem.key)}
    {#if loadErrors[subsystem.key]}
      <div
        class="notice bad"
        role="alert"
        aria-label={`${subsystem.label} load error`}
      >
        <strong>Failed to load {subsystem.label}</strong>
        <p>{loadErrors[subsystem.key]}</p>
        <Button
          variant="secondary"
          size="sm"
          disabled={busy}
          onclick={() => void work(refresh)}>Retry</Button
        >
      </div>
    {/if}
  {/each}
  <article class="service-detail" aria-label={`${definition.label} service`}>
    <aside class="service-rail" aria-label="Service controls">
      {#each definitions as service (service.kind)}
        {@const active = service.kind === selectedKind}
        {@const attached = integration(service)}
        {@const provisioned = provision(service.kind)}
        {@const runtimeService = runtime(service.kind)}
        {@const review = transferReviews[service.kind]}
        {@const container = containerFor(service)}
        {@const state = status(service.kind)}
        {@const url = externalUrl(service)}
        {@const progress = activity(service.kind)}
        <div class="rail-identity" class:inactive={!active} inert={!active}>
          <div class="rail-heading">
            <img src={icons[service.kind]} alt="" />
            <div>
              <span>{service.description}</span>
              <h2>{service.label}</h2>
            </div>
          </div>
          <span class="service-status" data-tone={state.tone}
            >{state.label}</span
          >
          {#if url}<a
              class="service-link"
              href={url}
              target="_blank"
              rel="noopener noreferrer"
              aria-label={`Open ${service.label}: ${url}`}
              ><span>{url.replace(/^https?:\/\//, '').replace(/\/$/, '')}</span
              ><ExternalLink size={13} aria-hidden="true" /></a
            >{/if}
          <div class="rail-progress">
            {#if progress}<span>{progress}</span>
              <div
                class="activity-bar"
                role="progressbar"
                aria-label={`${service.label}: ${progress}`}
              ></div>{/if}
          </div>
        </div>
        <div class="rail-actions" class:inactive={!active} inert={!active}>
          {#if review}
            <Button
              size="form"
              disabled={busy ||
                (!!review.compose_project && !releasedCompose[service.kind])}
              onclick={() =>
                void work(
                  () => adopt(service),
                  `${service.label} ownership transfer queued.`,
                )}>Take ownership</Button
            >
            <Button
              size="form"
              variant="secondary"
              disabled={busy}
              onclick={() => (transferReviews[service.kind] = null)}
              >Cancel</Button
            >
          {:else if provisioned?.state === 'blocked'}
            <Button
              size="form"
              disabled={busy}
              onclick={() => void work(() => retryProvision(service.kind))}
              >Retry setup</Button
            >
            {#if provisioned.origin === 'adopted' && !stackServices.some((entry) => entry.id === provisioned.id && !entry.transfer_pending)}<Button
                size="form"
                variant="secondary"
                disabled={busy}
                onclick={() => void work(() => restoreOriginal(service.kind))}
                >Restore original</Button
              >{/if}
          {:else if runtimeService && !setupActive(service.kind)}
            {#if runtimeService.existence === 'present'}
              {#each runtimeService.running ? ['restart', 'stop', 'reconcile'] : ['start', 'reconcile'] as action (action)}
                <Button
                  size="form"
                  variant="secondary"
                  disabled={!canControl(service.kind, action)}
                  onclick={() =>
                    void work(() => stackAction(service.kind, action))}
                  >{action === 'reconcile'
                    ? 'Repair configuration'
                    : action[0].toUpperCase() + action.slice(1)}</Button
                >
              {/each}
            {/if}
            {#if runtimeService.can_recreate}<Button
                size="form"
                disabled={busy}
                onclick={() =>
                  void work(() => stackAction(service.kind, 'recreate'))}
                >Recreate and start</Button
              >{/if}
          {:else if attached && !provisioned}
            <Button
              size="form"
              variant="secondary"
              disabled={busy || !!loadErrors.stack}
              onclick={() => void work(() => previewAdoption(service))}
              >Review ownership transfer</Button
            >
            {#if service.role === 'manager'}<Button
                size="form"
                variant="secondary"
                disabled={busy}
                onclick={() =>
                  void work(
                    () => testManager(service.kind),
                    'Connection and mount checks passed.',
                  )}>Test connection</Button
              >{/if}
          {/if}
          {#if provisioned && (runtimeService?.can_retire || (!runtimeService && ['blocked', 'retiring'].includes(provisioned.state)))}
            <Button
              size="form"
              variant="secondary"
              disabled={busy || setupActive(service.kind)}
              onclick={() =>
                void work(() => stackAction(service.kind, 'retire'))}
              >Retire and keep data</Button
            >
          {/if}
          {#if !attached && !provisioned && !runtimeService}<span
              class="text-xs text-muted">Choose how to connect.</span
            >{/if}
        </div>
        <dl class="rail-meta" class:inactive={!active} inert={!active}>
          <div>
            <dt>Ownership</dt>
            <dd>
              {runtimeService || provisioned
                ? 'Thelxinoe'
                : attached
                  ? 'External'
                  : 'Not configured'}
            </dd>
          </div>
          <div>
            <dt>Container</dt>
            <dd>
              {runtimeService?.name ||
                container?.names[0]?.replace(/^\//, '') ||
                (attached ? 'Unavailable' : '—')}
            </dd>
          </div>
          <div>
            <dt>Installed image</dt>
            <dd class="image-value">
              {runtimeService?.image || container?.image || '—'}
            </dd>
          </div>
          <div>
            <dt>Version</dt>
            <dd>{attached?.version || '—'}</dd>
          </div>
        </dl>
      {/each}
    </aside>
    <div class="service-workspace">
      {#if message}<p class="notice" role="status">{message}</p>{/if}
      <div class="workspace-tools">
        <Button
          variant="ghost"
          size="compact-icon"
          disabled={busy}
          title="Refresh services"
          aria-label="Refresh services"
          onclick={() => void work(refresh)}
          ><RefreshCw size={14} aria-hidden="true" /></Button
        >
      </div>
      {#if connected?.error || item?.error || live?.inspection_error || live?.error || live?.drift}
        <div class="notice warn" role="status">
          {#each [...new Set([connected?.error, item?.error, live?.inspection_error, live?.error].filter(Boolean))] as error (error)}<p
            >
              {error}
            </p>{/each}
          {#if live?.drift}<p>
              Configuration changed outside Thelxinoe. Repair configuration to
              restore managed settings.
            </p>{/if}
        </div>
      {/if}
      {#if transferReviews[selectedKind]}
        {@const review = transferReviews[selectedKind]!}
        <section class="work-section" aria-label="Ownership review">
          <h3>Ownership review</h3>
          <p>
            Thelxinoe stops the original container, disables its restart policy,
            copies its configuration, and starts the managed copy.
          </p>
          <dl class="review-paths">
            <div>
              <dt>Copy from</dt>
              <dd>{review.source_config}</dd>
            </div>
            <div>
              <dt>Copy to</dt>
              <dd>{review.managed_config}</dd>
            </div>
            <div>
              <dt>Retained image</dt>
              <dd>{review.image}</dd>
            </div>
          </dl>
          {#if review.compose_project}
            <p>
              Remove or disable <strong>{review.compose_service}</strong> in
              Compose project <strong>{review.compose_project}</strong> so it cannot
              recreate the original container.
            </p>
            <Switch size="sm" bind:checked={releasedCompose[selectedKind]}
              >The previous Compose definition is disabled</Switch
            >
          {:else}<p>
              Disable any external script or updater that would recreate the
              original container.
            </p>{/if}
        </section>
      {/if}
      {#if !connected && !item && !live}
        <section class="work-section" aria-label="Service setup">
          <h3>Set up {definition.label}</h3>
          <div class="setup-forms">
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
              <strong class="text-xs">Connect an existing container</strong>
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
        </section>
      {:else if setupActive(selectedKind)}
        <section class="work-section">
          <h3>{activity(selectedKind)}</h3>
          <p>
            This view refreshes automatically as the service becomes available.
          </p>
        </section>
      {/if}
      {#if definition.kind === 'nzbget' && nzbgetLoginTarget}
        <section class="work-section" aria-label="NZBGet login">
          <div class="work-head">
            <h3>NZBGet login</h3>
            {#if nzbgetLogin?.id === nzbgetLoginTarget.id}<Button
                variant="ghost"
                size="sm"
                onclick={() => {
                  nzbgetLogin = null;
                  showNzbgetPassword = false;
                }}>Hide login</Button
              >{:else}<Button
                variant="secondary"
                size="sm"
                disabled={busy}
                onclick={() => void work(revealNzbgetLogin)}>Show login</Button
              >{/if}
          </div>
          {#if nzbgetLogin && nzbgetLogin.id === nzbgetLoginTarget.id}
            <div class="login-fields">
              <label
                >Username<input readonly value={nzbgetLogin.username} /></label
              >
              <Button
                variant="secondary"
                size="sm"
                disabled={busy}
                onclick={() =>
                  void work(() =>
                    copyNzbgetLogin(nzbgetLogin!.username, 'Username'),
                  )}>Copy username</Button
              >
              <label
                >Password<input
                  readonly
                  type={showNzbgetPassword ? 'text' : 'password'}
                  value={nzbgetLogin.password}
                /></label
              >
              <div class="row-actions">
                <Button
                  variant="ghost"
                  size="sm"
                  onclick={() => (showNzbgetPassword = !showNzbgetPassword)}
                  >{showNzbgetPassword
                    ? 'Hide password'
                    : 'Show password'}</Button
                >
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busy}
                  onclick={() =>
                    void work(() =>
                      copyNzbgetLogin(nzbgetLogin!.password, 'Password'),
                    )}>Copy password</Button
                >
              </div>
            </div>
          {/if}
        </section>
      {/if}
      {#if connected}
        <section
          class="work-section"
          aria-label={definition.role === 'manager'
            ? 'Acquisition defaults'
            : definition.kind === 'prowlarr'
              ? 'Indexers'
              : definition.kind === 'nzbget'
                ? 'Downloads'
                : 'Missing subtitles'}
        >
          <div class="work-head">
            <h3>
              {definition.role === 'manager'
                ? 'Acquisition defaults'
                : definition.kind === 'prowlarr'
                  ? 'Indexers'
                  : definition.kind === 'nzbget'
                    ? 'Downloads'
                    : 'Missing subtitles'}
            </h3>
            {#if definition.role === 'manager'}<Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onclick={() =>
                  void work(() => loadManagerOptions(selectedKind))}
                >Reload choices</Button
              >{/if}
            {#if definition.role === 'manager' && (item || live)}
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onclick={() =>
                  void work(
                    () => testManager(selectedKind),
                    'Connection and mount checks passed.',
                  )}>Test connection</Button
              >
            {/if}
            {#if definition.kind === 'nzbget' && supportData}<Button
                variant="secondary"
                size="sm"
                disabled={busy}
                onclick={() =>
                  void work(() =>
                    supportCommand(
                      'nzbget',
                      supportData.paused ? 'resume_all' : 'pause_all',
                    ),
                  )}>{supportData.paused ? 'Resume all' : 'Pause all'}</Button
              >{/if}
          </div>
          {#if detailErrors[selectedKind]}<div class="notice bad" role="alert">
              {detailErrors[selectedKind]}
            </div>{/if}
          {#if detailLoading[selectedKind] && !supportData && !defaults[selectedKind]?.loaded}<p
              role="status"
            >
              Loading service settings…
            </p>{/if}
          {#if definition.role === 'manager'}
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
                      The canonical library folder will be created when these
                      defaults are saved.
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
                          <option value={option.id}>{option.name}</option>
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
          {:else if supportData}
            {#each supportData.health ?? [] as issue, index (index)}<p
                class="notice warn"
              >
                {typeof issue === 'string' ? issue : issue.message}
              </p>{/each}
            {#if selectedKind === 'prowlarr'}
              {#if !supportData.indexers?.length}<p>
                  No indexers configured. Open Prowlarr to add one.
                </p>{/if}
              {#each supportData.indexers ?? [] as indexer (indexer.id)}
                <div class="service-row">
                  <div>
                    <strong>{indexer.name}</strong><span class="row-status"
                      >{indexer.disabled_until
                        ? `Unavailable until ${indexer.disabled_until}`
                        : indexer.enabled
                          ? 'Enabled'
                          : 'Disabled'}</span
                    >
                  </div>
                  <div class="row-actions">
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={busy}
                      onclick={() =>
                        void work(() =>
                          supportCommand('prowlarr', 'test', {
                            item_id: indexer.id,
                          }),
                        )}>Test</Button
                    >
                    <Switch
                      size="sm"
                      checked={indexer.enabled}
                      disabled={busy}
                      onCheckedChange={(enabled) =>
                        void work(() =>
                          supportCommand(
                            'prowlarr',
                            enabled ? 'enable' : 'disable',
                            { item_id: indexer.id },
                          ),
                        )}
                      ><span class="sr-only">Enable {indexer.name}</span
                      ></Switch
                    >
                  </div>
                </div>
              {/each}
            {:else if selectedKind === 'nzbget'}
              <DownloadsTable
                queue={supportData.queue ?? []}
                history={supportData.history ?? []}
                paused={supportData.paused}
                {busy}
                onaction={(action, id) =>
                  void work(() =>
                    supportCommand('nzbget', action, { item_id: id }),
                  )}
              />
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
                  No missing subtitles reported. Configure language profiles and
                  providers in Bazarr.
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
                          item_id: missing.movie_id ?? missing.episode_id,
                          domain: missing.movie_id ? 'movies' : 'episodes',
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
        </section>
        {#if !loadErrors.connections}<ServiceConnections
            kind={selectedKind}
            {connections}
            {busy}
            onaction={(connection, action) =>
              void work(() => connectionAction(connection, action))}
          />{/if}
      {/if}
      {#if item && (live?.can_retire || (!live && ['blocked', 'retiring'].includes(item.state)))}<p
          class="notice"
        >
          Retiring removes the installation and its API connection. Appdata,
          request history, and media files are kept.
        </p>{/if}
      <section class="work-section" aria-label="Updates">
        <h3>Updates</h3>
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
                ><option value="inherit">Use server update policy</option
                ><option value="notify">Notify</option><option value="automatic"
                  >Automatic</option
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
                {String(serverPolicy.window_start).padStart(2, '0')}:00–{String(
                  serverPolicy.window_end,
                ).padStart(2, '0')}:00).
              </p>
            {/if}
            <Button type="submit" size="form" disabled={busy}
              >Save update policy</Button
            >
          </form>

          {#if policy?.candidate}<p class="candidate">
              Stable candidate: <code>{policy.candidate}</code>
            </p>{/if}
          {#if policy?.error}<p class="notice warn" role="status">
              {policy.error}
            </p>{/if}
          <div class="row-actions">
            <Button
              variant="secondary"
              size="form"
              disabled={busy}
              onclick={() => void work(() => preflight(selectedKind))}
              >Check compatibility</Button
            ><Button
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
          {#each serviceUpdatesList as update (update.id)}
            <div
              class="notice"
              class:warn={[
                'blocked',
                'recovery-required',
                'runtime-failure',
                'rolled-back',
              ].includes(update.state)}
              role="status"
            >
              <strong
                >{update.classification === 'incompatible'
                  ? 'Update incompatible'
                  : update.classification === 'unable-to-verify'
                    ? 'Unable to verify update'
                    : (stages[update.state] ?? update.state)}</strong
              >
              {#if update.error}<p>{update.error}</p>{/if}
              {#if update.state === 'ready'}<Button
                  size="sm"
                  disabled={busy}
                  onclick={() =>
                    void work(() => updateAction(update.id, 'activate'))}
                  >Install verified update</Button
                >{/if}
              {#if ['blocked', 'recovery-required'].includes(update.state)}<Button
                  variant="secondary"
                  size="sm"
                  disabled={busy}
                  onclick={() =>
                    void work(() => updateAction(update.id, 'recover'))}
                  >Recover before activation</Button
                >{/if}
              {#if update.state === 'runtime-failure'}<p>
                  Review the updated service before an explicit restore.
                </p>{/if}
            </div>
          {/each}
        {:else}<p>
            {connected
              ? 'Take ownership to manage updates here.'
              : 'Updates become available after managed setup.'}
          </p>{/if}
      </section>
    </div>
  </article>
  <footer class="services-footer">
    {#if approvalUsers.length}<section aria-label="Automatic request approval">
        <h3>Automatic request approval</h3>
        <div class="row-actions">
          {#each approvalUsers as user (user.id)}<Switch
              size="sm"
              checked={user.enabled}
              disabled={busy}
              onCheckedChange={(enabled) =>
                void work(async () => {
                  await api(`/admin/acquisition/users/${user.id}`, 'PUT', {
                    enabled,
                  });
                  await loadApprovalUsers();
                })}>{user.username}</Switch
            >{/each}
        </div>
      </section>{/if}
    <section>
      <Button
        variant="ghost"
        size="sm"
        disabled={busy || !!loadErrors.stack}
        onclick={() => void work(discoverReleases)}
        >Discover stable releases</Button
      >{#each releases as release (release.kind)}<p class="candidate">
          {release.kind}: {release.image ?? 'No stable release found'} · tested {release.tested_image}
        </p>{/each}
    </section>
  </footer>
</Panel>

<style>
  .service-strip {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    border-bottom: 1px solid var(--line);
    margin-bottom: 22px;
  }
  .service-tab {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 12px 8px;
    border: 0;
    border-right: 1px solid var(--line);
    border-bottom: 3px solid transparent;
    background: transparent;
    text-align: left;
    color: var(--foreground);
    cursor: pointer;
  }
  .service-tab:last-child {
    border-right: 0;
  }
  .service-tab:hover {
    background: var(--surface-soft);
  }
  .service-tab[aria-current='true'] {
    background: var(--accent-soft);
    border-bottom-color: var(--accent);
  }
  .service-tab img {
    width: 27px;
    height: 27px;
    object-fit: contain;
  }
  .service-tab strong {
    font-size: 11px;
    font-weight: 650;
  }
  .service-status {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 9px;
    color: var(--muted);
    line-height: 1.3;
    margin-top: 6px;
  }
  .service-status::before {
    content: '';
    width: 5px;
    height: 5px;
    flex: none;
    background: currentColor;
    border-radius: 50%;
  }
  .service-status[data-tone='ok'] {
    color: var(--success);
  }
  .service-status[data-tone='bad'] {
    color: var(--danger);
  }
  .service-status[data-tone='warn'] {
    color: var(--warning);
  }
  .service-status[data-tone='busy'] {
    color: var(--accent);
  }
  .service-status[data-tone='busy']::before {
    animation: pulse 1.5s ease-in-out infinite;
  }
  .service-detail {
    display: grid;
    grid-template-columns: minmax(175px, 25%) minmax(0, 1fr);
  }
  .service-rail {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows:
      minmax(180px, max-content) minmax(145px, max-content)
      minmax(215px, max-content);
    align-content: start;
    padding-right: 20px;
    border-right: 1px solid var(--line);
  }
  .rail-identity,
  .rail-actions,
  .rail-meta {
    grid-column: 1;
    min-width: 0;
    border-bottom: 1px solid var(--line);
    padding: 14px 0;
  }
  .inactive {
    visibility: hidden;
    pointer-events: none;
  }
  .rail-identity {
    grid-row: 1;
    display: flex;
    flex-direction: column;
    padding-top: 0;
  }
  .rail-heading {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .rail-heading img {
    width: 42px;
    height: 42px;
    object-fit: contain;
  }
  .rail-heading span {
    font-size: 9px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.07em;
  }
  .rail-heading h2 {
    font-size: 20px;
    margin: 2px 0 0;
  }
  .rail-identity > .service-status {
    margin-top: 15px;
    font-size: 11px;
  }
  .service-link {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-top: 12px;
    width: fit-content;
    max-width: 100%;
    font-size: 11px;
    color: var(--accent);
  }
  .service-link span {
    overflow-wrap: anywhere;
  }
  .service-link :global(svg) {
    flex: none;
  }
  .rail-progress {
    margin-top: auto;
    padding-top: 12px;
    min-height: 36px;
    font-size: 10px;
    color: var(--muted);
  }
  .activity-bar {
    position: relative;
    height: 3px;
    overflow: hidden;
    background: var(--accent-soft);
    margin-top: 7px;
  }
  .activity-bar::after {
    content: '';
    position: absolute;
    height: 100%;
    width: 40%;
    left: -40%;
    background: var(--accent);
    animation: travel 1.8s ease-in-out infinite;
  }
  .rail-actions {
    grid-row: 2;
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
    gap: 7px;
  }
  .rail-actions :global(button) {
    width: 100%;
    white-space: normal;
  }
  .rail-meta {
    grid-row: 3;
    border-bottom: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  dt {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--muted);
  }
  dd {
    font-size: 11px;
    margin: 3px 0 0;
    overflow-wrap: anywhere;
  }
  .image-value {
    font-size: 10px;
  }
  .service-workspace {
    position: relative;
    padding-left: 22px;
    min-width: 0;
  }
  .workspace-tools {
    display: flex;
    justify-content: flex-end;
    height: 0;
    transform: translateY(-7px);
  }
  .workspace-tools :global(button) {
    z-index: 1;
  }
  .work-section {
    padding: 16px 0;
    border-bottom: 1px solid var(--line);
  }
  .work-section:first-of-type {
    padding-top: 0;
  }
  .work-section:last-child {
    border-bottom: 0;
  }
  h3 {
    font-size: 12px;
    font-weight: 650;
    margin: 0 0 13px;
  }
  .work-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    margin: 0 28px 12px 0;
  }
  .work-head h3 {
    margin: 0;
  }
  .login-fields {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: end;
    gap: 10px;
  }
  .login-fields label {
    min-width: 0;
  }
  .login-fields input {
    width: 100%;
  }
  .service-workspace p,
  .services-footer p {
    color: var(--muted);
    font-size: 11px;
    line-height: 1.6;
    margin: 8px 0;
    overflow-wrap: anywhere;
  }
  .notice {
    border-left: 3px solid var(--accent);
    background: var(--accent-soft);
    padding: 10px 12px;
    font-size: 11px;
    margin: 0 0 13px;
    overflow-wrap: anywhere;
  }
  .notice.warn {
    border-color: var(--warning);
  }
  .notice.bad {
    border-color: var(--danger);
  }
  .notice p {
    margin: 3px 0;
  }
  .notice :global(button) {
    margin-top: 8px;
  }
  .service-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    border-bottom: 1px solid var(--line);
    padding: 12px 0;
    font-size: 11px;
  }
  .service-row > div {
    min-width: 0;
  }
  .row-status {
    display: block;
    color: var(--muted);
    font-size: 10px;
    margin-top: 4px;
  }
  .row-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .review-paths {
    display: grid;
    gap: 12px;
    margin: 15px 0;
  }
  .setup-forms {
    display: grid;
    gap: 22px;
  }
  .service-workspace :global(form > button) {
    width: fit-content;
  }
  .service-workspace :global(label) {
    font-size: 11px;
  }
  .service-workspace :global(input),
  .service-workspace :global(select) {
    min-width: 0;
  }
  .candidate {
    overflow-wrap: anywhere;
  }
  .services-footer {
    border-top: 1px solid var(--line);
    margin-top: 24px;
    padding-top: 18px;
    display: grid;
    gap: 20px;
  }
  @keyframes travel {
    to {
      left: 100%;
    }
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
  @media (max-width: 1050px) {
    .service-tab {
      flex-direction: column;
      align-items: flex-start;
    }
    .service-detail {
      grid-template-columns: minmax(160px, 27%) minmax(0, 1fr);
    }
  }
  @media (max-width: 720px) {
    .service-strip {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    .service-tab {
      flex-direction: row;
    }
    .service-detail {
      grid-template-columns: minmax(0, 1fr);
    }
    .service-rail {
      display: block;
      padding: 0;
      border: 0;
    }
    .service-rail > .inactive {
      display: none;
    }
    .rail-identity {
      min-height: 170px;
    }
    .rail-actions {
      flex-direction: row;
      flex-wrap: wrap;
    }
    .rail-actions :global(button) {
      width: auto;
    }
    .rail-meta {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      border-bottom: 1px solid var(--line);
    }
    .service-workspace {
      padding: 22px 0 0;
    }
    .work-head {
      flex-wrap: wrap;
    }
  }
  @media (max-width: 440px) {
    .service-strip {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .service-tab {
      padding: 10px 5px;
      gap: 5px;
    }
    .service-tab img {
      width: 23px;
      height: 23px;
    }
    .rail-meta {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .activity-bar::after,
    .service-status[data-tone='busy']::before {
      animation: none;
    }
    .activity-bar::after {
      left: 30%;
    }
  }
</style>
