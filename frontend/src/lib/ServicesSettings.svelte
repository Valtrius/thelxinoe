<script lang="ts">
  import FormField from './ui/FormField.svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import UpdatePolicyFields from './ui/UpdatePolicyFields.svelte';
  import { formControlClass } from './ui/styles';
  import StatusIndicator from './ui/StatusIndicator.svelte';
  import Notice from './ui/Notice.svelte';
  import ProgressBar from './ui/ProgressBar.svelte';
  import { onMount } from 'svelte';
  import { api, serverUrl } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import ConfirmDialog from './providers/components/ui/ConfirmDialog.svelte';
  import Switch from './ui/Switch.svelte';
  import { Copy, ExternalLink, LoaderCircle } from '@lucide/svelte';
  import DownloadsTable from './services/DownloadsTable.svelte';
  import type { SupportDownload } from './services/downloads';
  import { serviceUiUrl, type Container } from './services/presentation';
  import radarrIcon from './services/icons/radarr.png';
  import sonarrIcon from './services/icons/sonarr.png';
  import lidarrIcon from './services/icons/lidarr.png';
  import bazarrIcon from './services/icons/bazarr.png';
  import prowlarrIcon from './services/icons/prowlarr.png';
  import nzbgetIcon from './services/icons/nzbget.png';
  import seerrIcon from './services/icons/seerr.png';
  import QualityProfileEditor from './services/QualityProfileEditor.svelte';
  import IndexerOnboarding from './services/IndexerOnboarding.svelte';
  import ServiceConnections, {
    type ServiceConnection,
  } from './ServiceConnections.svelte';

  let { timeFormat = '24h' } = $props<{ timeFormat?: '12h' | '24h' }>();

  type ServiceKind =
    'radarr' | 'sonarr' | 'lidarr' | 'bazarr' | 'prowlarr' | 'nzbget' | 'seerr';
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
    defaults: ManagerDefaults;
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
    initialized?: boolean;
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
    registered?: boolean;
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
    can_remove: boolean;
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
    checked_at: number;
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
      kind: 'seerr',
      label: 'Seerr',
      role: 'support',
      internalPort: 5055,
      hostPort: 15055,
      description: 'Discovery and requests',
    },
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
    approvalUsers = $state<ApprovalUser[]>([]);
  let timezone = $state(''),
    serverPolicy = $state<ServerUpdatePolicy>({
      policy: 'notify',
      window_start: 3,
      window_end: 5,
    });
  const pendingActions = $state<
    Partial<Record<ServiceKind | 'general', boolean>>
  >({});
  const copyingCredentials = $state<
    Partial<Record<'username' | 'password', boolean>>
  >({});
  const feedback = $state<Partial<Record<ServiceKind | 'general', string>>>({});
  const connectionTests = $state<Partial<Record<ServiceKind, string>>>({});
  const connectionErrors = $state<Partial<Record<ServiceKind, string>>>({});
  let removalKind = $state<ServiceKind | null>(null);
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
    seerr: seerrIcon,
  };
  let selectedKind = $state<ServiceKind>('seerr');
  const busy = $derived(isBusy(selectedKind));
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
    if (state === 'retiring' && !pendingActions[kind] && feedback[kind])
      return '';
    if (setupActive(kind))
      return (
        {
          queued: 'Waiting to install',
          installing: 'Installing',
          connecting: 'Connecting API',
          retiring: 'Retiring installation',
        } as Record<string, string>
      )[state!];
    const update = activeUpdate(kind);
    if (update)
      return update.state === 'activating'
        ? 'Connecting API'
        : stages[update.state];
    return pendingActions[kind] ? 'Working' : '';
  }
  function activeUpdate(kind: ServiceKind) {
    return serviceUpdates(kind)
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
  }
  function isBusy(kind: ServiceKind, retryRemoval = false) {
    return (
      !!pendingActions[kind] ||
      (!(retryRemoval && provision(kind)?.state === 'retiring') &&
        setupActive(kind)) ||
      !!activeUpdate(kind)
    );
  }
  function status(kind: ServiceKind) {
    const item = provision(kind),
      live = runtime(kind);
    if (live?.registered === false)
      return { label: 'Setup mismatch', tone: 'warn' };
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
      !isBusy(kind) &&
      !setupActive(kind) &&
      !!live &&
      live.registered !== false &&
      live.existence !== 'unknown' &&
      (action === 'reconcile' ||
        (live.existence === 'present' &&
          !live.drift &&
          live.phase === 'active'))
    );
  }
  async function loadSelectedData() {
    const kind = selectedKind;
    if (detailLoading[kind] || isBusy(kind)) return;
    detailLoading[kind] = true;
    detailErrors[kind] = '';
    try {
      if (manager(kind) && runtime(kind)?.running !== false)
        await loadManagerOptions(kind);
      else if (supportService(kind)) await refreshSupport(kind);
    } catch (error) {
      detailErrors[kind] = String(error);
    } finally {
      detailLoading[kind] = false;
    }
  }
  function selectService(kind: ServiceKind) {
    selectedKind = kind;
    removalKind = null;
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
  let updateLoad = 0;
  async function loadUpdateData() {
    const request = ++updateLoad;
    loadErrors.updates = '';
    try {
      const result = await api<{
        policies: UpdatePolicy[];
        timezone: string;
        items: Update[];
        services: UpdateTarget[];
        server_policy: ServerUpdatePolicy;
      }>('/admin/service-updates');
      if (request !== updateLoad) return;
      policies = result.policies;
      updates = result.items;
      updateTargets = result.services;
      timezone = result.timezone;
      serverPolicy = result.server_policy;
      syncUpdateDrafts();
    } catch (error) {
      if (request === updateLoad) loadErrors.updates = String(error);
    }
  }
  async function loadManagers() {
    loadErrors.managers = '';
    try {
      managers = (await api<{ items: ManagerService[] }>('/admin/managers'))
        .items;
      for (const definition of definitions) {
        const draft = defaults[definition.kind];
        if (draft && !manager(definition.kind)) draft.loaded = false;
      }
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
        can_remove: false,
      }));
    }
  }
  let pendingRefresh: Promise<void> | undefined;
  async function refresh() {
    // Serialize background reads with action refreshes so an older response
    // cannot restore a service that has just been removed.
    const previous = pendingRefresh;
    const current = (async () => {
      await previous;
      await refreshData();
    })();
    pendingRefresh = current;
    try {
      await current;
    } finally {
      if (pendingRefresh === current) pendingRefresh = undefined;
    }
  }
  async function refreshData() {
    await Promise.all([
      loadManagers(),
      loadSupport(),
      loadApprovalUsers(),
      loadUpdateData(),
      loadStack(),
      loadConnections(),
    ]);
    await loadContainers();
    void loadSelectedData();
  }
  let connectionLoad = 0;
  async function loadConnections() {
    const request = ++connectionLoad;
    loadErrors.connections = '';
    try {
      const result = await api<{ items: ServiceConnection[] }>(
        '/admin/service-connections',
      );
      if (request === connectionLoad) connections = result.items;
    } catch (error) {
      if (request === connectionLoad) loadErrors.connections = String(error);
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
  async function work(
    action: () => Promise<void>,
    success = '',
    kind: ServiceKind | 'general' = selectedKind,
    allowBackground = false,
  ) {
    if (
      kind === 'general' || allowBackground
        ? pendingActions[kind]
        : isBusy(kind)
    )
      return;
    pendingActions[kind] = true;
    feedback[kind] = '';
    try {
      await action();
      if (success) feedback[kind] = success;
    } catch (error) {
      feedback[kind] = String(error);
    } finally {
      pendingActions[kind] = false;
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
    if (manager(kind)?.id !== service.id) return;
    managerOptions[kind] = options;
    if (draft.loaded) return;
    service.defaults = options.defaults ?? service.defaults;
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
  async function saveManagerDefaults(
    kind: ServiceKind,
    value: ManagerDefaults,
  ) {
    const service = manager(kind);
    if (!service) return;
    await api(`/admin/managers/${service.id}/defaults`, 'PUT', value);
    service.defaults = value;
  }
  async function testManager(kind: ServiceKind) {
    const service = manager(kind);
    if (!service) return;
    connectionTests[kind] = 'Testing…';
    connectionErrors[kind] = '';
    try {
      await api(`/admin/managers/${service.id}/test`, 'POST');
      connectionTests[kind] = 'Connection OK';
    } catch (error) {
      connectionTests[kind] = 'Test failed';
      connectionErrors[kind] = String(error);
    }
  }
  async function refreshSupport(kind: ServiceKind) {
    const service = supportService(kind);
    if (!service) return;
    const snapshot = await api<SupportSnapshot>(`/admin/support/${service.id}`);
    if (supportService(kind)?.id === service.id) snapshots[kind] = snapshot;
  }
  async function copyNzbgetCredential(field: 'username' | 'password') {
    const service = nzbgetLoginTarget;
    if (!service) return;
    const source = provision('nzbget') ? 'stack' : 'support';
    const login = await api<{ username: string; password: string }>(
      `/admin/${source}/${service.id}/login`,
      'POST',
    );
    const value = login[field];
    if (navigator.clipboard?.writeText) {
      try {
        await navigator.clipboard.writeText(value);
        return;
      } catch {
        // Local HTTP sessions can use the selection-based fallback below.
      }
    }
    const input = document.createElement('textarea');
    input.value = value;
    input.className = 'fixed -left-[9999px]';
    document.body.appendChild(input);
    let copied: boolean;
    try {
      input.select();
      copied = document.execCommand('copy');
    } finally {
      input.remove();
    }
    if (!copied) throw new Error('Could not copy the NZBGet credential.');
  }
  async function copyNzbgetCredentialFromButton(
    field: 'username' | 'password',
  ) {
    if (copyingCredentials[field]) return;
    copyingCredentials[field] = true;
    feedback.nzbget = '';
    try {
      await copyNzbgetCredential(field);
    } catch (error) {
      feedback.nzbget = String(error);
    } finally {
      copyingCredentials[field] = false;
    }
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
    if (action === 'remove') {
      const draft = defaults[kind];
      if (draft) draft.loaded = false;
      delete managerOptions[kind];
      delete snapshots[kind];
      delete connectionTests[kind];
    }
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

  onMount(() => {
    void refresh();
    let polling = false;
    const poll = async () => {
      if (polling || document.hidden) return;
      polling = true;
      try {
        await refresh();
      } finally {
        polling = false;
      }
    };
    const timer = setInterval(() => void poll(), 4000);
    document.addEventListener('visibilitychange', poll);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', poll);
    };
  });
</script>

<Panel
  aria-label="Media services"
  class="services-panel settings-panel:compact:p-0"
>
  <ConfirmDialog
    open={removalKind !== null}
    title={`Remove ${definitions.find((service) => service.kind === removalKind)?.label ?? 'service'}?`}
    message="Delete this service's container, configuration, and retained update copies. Its connections in other services will be removed automatically. Media files and other services are kept."
    confirmLabel="Remove service and configuration"
    eyebrow="REMOVE / MEDIA SERVICE"
    danger
    busy={removalKind ? isBusy(removalKind, true) : false}
    error={removalKind ? (feedback[removalKind] ?? '') : ''}
    onCancel={() => (removalKind = null)}
    onConfirm={() => {
      const kind = removalKind;
      removalKind = null;
      if (kind) void work(() => stackAction(kind, 'remove'), '', kind, true);
    }}
  />
  <nav
    class="service-strip mb-5.5 grid grid-cols-[repeat(7,minmax(8.5rem,1fr))] overflow-x-auto border-b border-line"
    aria-label="Select service"
  >
    {#each definitions as service (service.kind)}
      {@const state = status(service.kind)}
      <button
        class="service-tab flex min-w-0 cursor-pointer items-center gap-2 border-0 border-r border-b-3 border-r-line border-b-transparent bg-transparent px-2 py-3 text-left text-foreground last:border-r-0 hover:bg-surface-soft aria-[current=true]:border-b-accent aria-[current=true]:bg-accent-soft tight:gap-1.25 tight:px-1.25 tight:py-2.5 [&>img]:size-6.75 [&>img]:object-contain tight:[&>img]:size-5.75 [&_strong]:text-[11px] [&_strong]:font-[650]"
        aria-current={selectedKind === service.kind ? 'true' : undefined}
        aria-label={service.label}
        onclick={() => selectService(service.kind)}
      >
        <img src={icons[service.kind]} alt="" />
        <span
          ><strong>{service.label}</strong><StatusIndicator
            class="service-status mt-1.5"
            tone={state.tone}>{state.label}</StatusIndicator
          ></span
        >
      </button>
    {/each}
  </nav>
  {#each loaderSubsystems as subsystem (subsystem.key)}
    {#if loadErrors[subsystem.key]}
      <Notice
        tone="danger"
        role="alert"
        aria-label={`${subsystem.label} load error`}
      >
        <strong>Failed to load {subsystem.label}</strong>
        <p>{loadErrors[subsystem.key]}</p>
        <Button
          variant="secondary"
          size="sm"
          disabled={pendingActions.general}
          onclick={() => void work(refresh, '', 'general')}>Retry</Button
        >
      </Notice>
    {/if}
  {/each}
  <article
    class="service-detail grid grid-cols-[minmax(175px,25%)_minmax(0,1fr)] service-narrow:grid-cols-[minmax(160px,27%)_minmax(0,1fr)] compact:grid-cols-1"
    aria-label={`${definition.label} service`}
  >
    <aside
      class="service-rail grid grid-cols-1 grid-rows-[minmax(180px,max-content)_minmax(145px,max-content)_minmax(215px,max-content)] content-start border-r border-line pr-5 pl-4 compact:block compact:border-0 compact:px-3"
      aria-label="Service controls"
    >
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
        <div
          class={[
            'rail-identity col-start-1 min-w-0 border-b border-line py-3.5 row-start-1 flex flex-col pt-0 compact:min-h-42.5',
            !active && 'inactive invisible pointer-events-none compact:hidden',
          ]}
          inert={!active}
        >
          <div
            class="rail-heading flex items-center gap-2.5 [&_img]:size-10.5 [&_img]:object-contain [&_span]:text-[9px] [&_span]:tracking-[0.07em] [&_span]:text-muted [&_span]:uppercase [&_h2]:mt-0.5 [&_h2]:mb-0 [&_h2]:text-[20px]"
          >
            <img src={icons[service.kind]} alt="" />
            <div>
              <span>{service.description}</span>
              <h2>{service.label}</h2>
            </div>
          </div>
          <StatusIndicator
            class="service-status mt-3.75 text-[11px]"
            tone={state.tone}>{state.label}</StatusIndicator
          >
          {#if url}<div
              class="service-url-line mt-3 flex min-h-4.5 min-w-0 items-center gap-1"
            >
              <a
                class="service-link flex w-fit max-w-full min-w-0 flex-[0_1_auto] items-center gap-1.25 text-[11px] text-accent [&_span]:min-w-0 [&_span]:truncate [&_svg]:shrink-0"
                href={url}
                target="_blank"
                rel="noopener noreferrer"
                aria-label={`Open ${service.label}: ${url}`}
                ><span
                  >{url.replace(/^https?:\/\//, '').replace(/\/$/, '')}</span
                ><ExternalLink size={13} aria-hidden="true" /></a
              >
              {#if service.kind === 'nzbget' && nzbgetLoginTarget}
                <div class="ml-auto flex shrink-0 items-center gap-1">
                  <Button
                    variant="secondary"
                    size="credential"
                    class="credential-copy"
                    type="button"
                    aria-label="Copy NZBGet login"
                    title="Copy NZBGet login"
                    aria-busy={copyingCredentials.username}
                    disabled={pendingActions.nzbget ||
                      copyingCredentials.username}
                    onclick={() =>
                      void copyNzbgetCredentialFromButton('username')}
                    >{#if copyingCredentials.username}<LoaderCircle
                        size={10}
                        class="animate-spin motion-reduce:animate-none"
                        aria-hidden="true"
                      />{:else}<Copy
                        size={10}
                        aria-hidden="true"
                      />{/if}Login</Button
                  >
                  <Button
                    variant="secondary"
                    size="credential"
                    class="credential-copy"
                    type="button"
                    aria-label="Copy NZBGet password"
                    title="Copy NZBGet password"
                    aria-busy={copyingCredentials.password}
                    disabled={pendingActions.nzbget ||
                      copyingCredentials.password}
                    onclick={() =>
                      void copyNzbgetCredentialFromButton('password')}
                    >{#if copyingCredentials.password}<LoaderCircle
                        size={10}
                        class="animate-spin motion-reduce:animate-none"
                        aria-hidden="true"
                      />{:else}<Copy
                        size={10}
                        aria-hidden="true"
                      />{/if}Pass</Button
                  >
                </div>
              {/if}
            </div>{/if}
          <div
            class="rail-progress mt-auto min-h-9 pt-3 text-[10px] text-muted"
          >
            {#if progress}<span>{progress}</span>
              <ProgressBar
                class="activity-bar mt-1.75 h-0.75"
                indeterminate
                label={`${service.label}: ${progress}`}
              ></ProgressBar>{:else if feedback[service.kind]}<span
                class="action-feedback block truncate"
                role="status"
                title={feedback[service.kind]}>{feedback[service.kind]}</span
              >{/if}
          </div>
        </div>
        <div
          class={[
            'rail-actions col-start-1 min-w-0 border-b border-line py-3.5 row-start-2 flex flex-col justify-start gap-1.75 [&_button]:w-full [&_button]:whitespace-normal compact:flex-row compact:flex-wrap compact:[&_button]:w-auto',
            !active && 'inactive invisible pointer-events-none compact:hidden',
          ]}
          inert={!active}
        >
          {#if review}
            <Button
              size="form"
              disabled={isBusy(service.kind) ||
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
              disabled={isBusy(service.kind)}
              onclick={() => (transferReviews[service.kind] = null)}
              >Cancel</Button
            >
          {:else if provisioned?.state === 'blocked'}
            <Button
              size="form"
              disabled={isBusy(service.kind)}
              onclick={() => void work(() => retryProvision(service.kind))}
              >Retry setup</Button
            >
            {#if provisioned.origin === 'adopted' && !stackServices.some((entry) => entry.id === provisioned.id && !entry.transfer_pending)}<Button
                size="form"
                variant="secondary"
                disabled={isBusy(service.kind)}
                onclick={() => void work(() => restoreOriginal(service.kind))}
                >Restore original</Button
              >{/if}
          {:else if runtimeService && runtimeService.registered !== false && !setupActive(service.kind)}
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
                disabled={isBusy(service.kind)}
                onclick={() =>
                  void work(() => stackAction(service.kind, 'recreate'))}
                >Recreate and start</Button
              >{/if}
          {:else if attached && !provisioned}
            <Button
              size="form"
              variant="secondary"
              disabled={isBusy(service.kind) || !!loadErrors.stack}
              onclick={() => void work(() => previewAdoption(service))}
              >Review ownership transfer</Button
            >
            {#if service.role === 'manager'}<Button
                size="form"
                variant="secondary"
                disabled={isBusy(service.kind)}
                onclick={() => void work(() => testManager(service.kind))}
                title={connectionErrors[service.kind] ||
                  attached?.error ||
                  undefined}
                ><span
                  class="connection-result inline-block w-25"
                  aria-live="polite"
                  >{connectionTests[service.kind] || 'Test connection'}</span
                ></Button
              >{/if}
          {/if}
          {#if provisioned && (runtimeService?.can_retire || (!runtimeService && ['blocked', 'retiring'].includes(provisioned.state)))}
            <Button
              size="form"
              variant="secondary"
              disabled={isBusy(service.kind) || setupActive(service.kind)}
              onclick={() =>
                void work(() => stackAction(service.kind, 'retire'))}
              >Retire and keep data</Button
            >
          {/if}
          {#if provisioned && (runtimeService?.can_remove || provisioned.state === 'retiring')}
            <Button
              size="form"
              variant="secondary"
              disabled={isBusy(service.kind, true)}
              onclick={() => {
                feedback[service.kind] = '';
                removalKind = service.kind;
              }}>Remove service</Button
            >
          {/if}
          {#if !attached && !provisioned && !runtimeService}<span
              class="text-xs text-muted">Choose how to connect.</span
            >{/if}
        </div>
        <dl
          class={[
            'rail-meta col-start-1 min-w-0 border-b border-line py-3.5 row-start-3 m-0 flex flex-col gap-3 border-b-0 compact:grid compact:grid-cols-2 compact:border-b tight:grid-cols-1',
            !active && 'inactive invisible pointer-events-none compact:hidden',
          ]}
          inert={!active}
        >
          <div>
            <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
              Ownership
            </dt>
            <dd class="mt-0.75 wrap-anywhere text-[11px]">
              {runtimeService || provisioned
                ? 'Thelxinoe'
                : attached
                  ? 'External'
                  : 'Not configured'}
            </dd>
          </div>
          <div>
            <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
              Container
            </dt>
            <dd class="mt-0.75 wrap-anywhere text-[11px]">
              {runtimeService?.name ||
                container?.names[0]?.replace(/^\//, '') ||
                (attached ? 'Unavailable' : '—')}
            </dd>
          </div>
          <div>
            <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
              Installed image
            </dt>
            <dd class="image-value mt-0.75 wrap-anywhere text-[10px]">
              {runtimeService?.image || container?.image || '—'}
            </dd>
          </div>
          <div>
            <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
              Version
            </dt>
            <dd class="mt-0.75 wrap-anywhere text-[11px]">
              {attached?.version || '—'}
            </dd>
          </div>
        </dl>
      {/each}
    </aside>
    <div
      class="service-workspace relative min-w-0 pl-5.5 compact:px-3 compact:pt-5.5 [&_form>button]:w-fit"
    >
      {#if live?.registered === false}
        <div
          class="mb-3 border-l-2 border-warning bg-surface px-3 py-2 text-xs leading-5"
          role="status"
          aria-label="Service setup mismatch"
        >
          Saved service data has no matching record in this server profile.
          Restore the matching server profile to manage it. For a new instance,
          use fresh storage for both the server and its controller.
        </div>
      {/if}
      {#if item?.error || live?.inspection_error || live?.error || live?.drift}
        <Notice tone="warning" role="status">
          {#each [...new Set([item?.error, live?.inspection_error, live?.error].filter(Boolean))] as error (error)}<p
              class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
            >
              {error}
            </p>{/each}
          {#if live?.drift}<p
              class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
            >
              Configuration changed outside Thelxinoe. Repair configuration to
              restore managed settings.
            </p>{/if}
        </Notice>
      {/if}
      {#if transferReviews[selectedKind]}
        {@const review = transferReviews[selectedKind]!}
        <section
          class="work-section border-b border-line py-4 first-of-type:pt-0 last:border-b-0"
          aria-label="Ownership review"
        >
          <h3 class="mb-3.25 text-[12px] font-[650]">Ownership review</h3>
          <p class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted">
            Thelxinoe stops the original container, disables its restart policy,
            copies its configuration, and starts the managed copy.
          </p>
          <dl class="review-paths my-3.75 grid gap-3">
            <div>
              <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
                Copy from
              </dt>
              <dd class="mt-0.75 wrap-anywhere text-[11px]">
                {review.source_config}
              </dd>
            </div>
            <div>
              <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
                Copy to
              </dt>
              <dd class="mt-0.75 wrap-anywhere text-[11px]">
                {review.managed_config}
              </dd>
            </div>
            <div>
              <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
                Retained image
              </dt>
              <dd class="mt-0.75 wrap-anywhere text-[11px]">{review.image}</dd>
            </div>
          </dl>
          {#if review.compose_project}
            <p class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted">
              Remove or disable <strong>{review.compose_service}</strong> in
              Compose project <strong>{review.compose_project}</strong> so it cannot
              recreate the original container.
            </p>
            <Switch size="sm" bind:checked={releasedCompose[selectedKind]}
              >The previous Compose definition is disabled</Switch
            >
          {:else}<p
              class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
            >
              Disable any external script or updater that would recreate the
              original container.
            </p>{/if}
        </section>
      {/if}
      {#if !connected && !item && !live}
        <section
          class="work-section border-b border-line py-4 first-of-type:pt-0 last:border-b-0"
          aria-label="Service setup"
        >
          <h3 class="mb-3.25 text-[12px] font-[650]">
            Set up {definition.label}
          </h3>
          <div class="setup-forms grid gap-5.5">
            <form
              class="grid gap-3 [&_label]:m-0"
              aria-label={`Install managed ${definition.label}`}
              onsubmit={(event) => {
                event.preventDefault();
                void work(() => installManaged(definition));
              }}
            >
              <strong class="text-xs">Install and own it</strong>
              <div class="grid grid-cols-2 gap-3 compact:grid-cols-1">
                <FormField
                  >Local port<input
                    class={formControlClass}
                    type="number"
                    min="1024"
                    max="65535"
                    bind:value={installs[definition.kind].hostPort}
                    required
                  /></FormField
                >
                <FormField
                  >Advanced UI address<input
                    class={formControlClass}
                    type="url"
                    bind:value={installs[definition.kind].nativeUrl}
                    placeholder="Optional"
                  /></FormField
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
              <FormField
                >Name<input
                  class={formControlClass}
                  bind:value={setup[definition.kind].name}
                  required
                  maxlength="100"
                /></FormField
              >
              <div class="grid grid-cols-2 gap-3 compact:grid-cols-1">
                <FormField
                  >Container<select
                    class={formControlClass}
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
                  ></FormField
                >
                <FormField
                  >Internal port<input
                    class={formControlClass}
                    type="number"
                    min="1"
                    max="65535"
                    bind:value={setup[definition.kind].port}
                    required
                  /></FormField
                >
              </div>
              {#if definition.kind === 'nzbget'}
                <FormField
                  >NZBGet username<input
                    class={formControlClass}
                    bind:value={setup.nzbget.username}
                    required
                    autocomplete="off"
                  /></FormField
                >
              {/if}
              <FormField
                >{definition.kind === 'nzbget'
                  ? 'NZBGet password'
                  : 'API key'}<input
                  class={formControlClass}
                  type="password"
                  bind:value={setup[definition.kind].secret}
                  required
                  autocomplete="new-password"
                /></FormField
              >
              {#if definition.role === 'support'}
                <FormField
                  >Service UI address<input
                    class={formControlClass}
                    type="url"
                    bind:value={setup[definition.kind].nativeUrl}
                    placeholder="https://service.example.com"
                  /></FormField
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
        <section
          class="work-section border-b border-line py-4 first-of-type:pt-0 last:border-b-0"
        >
          <h3 class="mb-3.25 text-[12px] font-[650]">
            {activity(selectedKind)}
          </h3>
          <p class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted">
            This view refreshes automatically as the service becomes available.
          </p>
        </section>
      {/if}
      {#if connected}
        <section
          class="work-section border-b border-line py-4 first-of-type:pt-0 last:border-b-0"
          aria-label={definition.role === 'manager'
            ? 'Acquisition defaults'
            : definition.kind === 'seerr'
              ? 'Requests'
              : definition.kind === 'prowlarr'
                ? 'Indexers'
                : definition.kind === 'nzbget'
                  ? 'Downloads'
                  : 'Missing subtitles'}
        >
          <div
            class="work-head mt-0 mr-7 mb-3 ml-0 flex items-center justify-between gap-3 compact:flex-wrap [&_h3]:m-0"
          >
            <h3 class="mb-3.25 text-[12px] font-[650]">
              {definition.role === 'manager'
                ? 'Acquisition defaults'
                : definition.kind === 'seerr'
                  ? 'Requests'
                  : definition.kind === 'prowlarr'
                    ? 'Indexers'
                    : definition.kind === 'nzbget'
                      ? 'Downloads'
                      : 'Missing subtitles'}
            </h3>
            {#if definition.role === 'manager' && (item || live)}
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onclick={() => void work(() => testManager(selectedKind))}
                title={connectionErrors[selectedKind] ||
                  connected?.error ||
                  undefined}
                ><span
                  class="connection-result inline-block w-25"
                  aria-live="polite"
                  >{connectionTests[selectedKind] || 'Test connection'}</span
                ></Button
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
          {#if detailErrors[selectedKind]}<Notice tone="danger" role="alert">
              {detailErrors[selectedKind]}
            </Notice>{/if}
          {#if detailLoading[selectedKind] && !supportData && !defaults[selectedKind]?.loaded}<p
              class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
              role="status"
            >
              Loading service settings…
            </p>{/if}
          {#if definition.role === 'manager'}
            {#if defaults[definition.kind]?.loaded}
              {@const options = managerOptions[definition.kind]}
              {@const draft = defaults[definition.kind]!}
              {#if options}
                {#key connected.id}<AutoSaveForm
                    class="mt-3 grid gap-3 [&_label]:m-0"
                    label={`${definition.label} acquisition defaults`}
                    value={{
                      root_folder: draft.root_folder,
                      quality_profile: draft.quality_profile,
                      metadata_profile: draft.metadata_profile,
                      monitored: draft.monitored,
                    }}
                    onsave={(value) =>
                      saveManagerDefaults(definition.kind, value)}
                    onRevert={(value) => Object.assign(draft, value)}
                    disabled={busy}
                  >
                    <p class="text-[11px] text-muted">
                      Library folder: <code>{draft.root_folder}</code>
                    </p>
                    <FormField
                      >Quality profile<select
                        class={formControlClass}
                        bind:value={draft.quality_profile}
                        required
                        >{#each options.profiles as option (option.id)}<option
                            value={option.id}>{option.name}</option
                          >{/each}</select
                      ></FormField
                    >
                    {#if definition.kind === 'lidarr'}<FormField
                        >Metadata profile<select
                          class={formControlClass}
                          bind:value={draft.metadata_profile}
                          required
                          >{#each options.metadata_profiles as option (option.id)}<option
                              value={option.id}>{option.name}</option
                            >{/each}</select
                        ></FormField
                      >{/if}
                    <Switch bind:checked={draft.monitored} size="sm"
                      >Monitor and search requests</Switch
                    >
                  </AutoSaveForm>{/key}
                {#if ['radarr', 'sonarr'].includes(definition.kind)}<div
                    class="mt-4"
                  >
                    {#key connected.id}<QualityProfileEditor
                        serviceId={connected.id}
                        created={async () => {
                          draft.loaded = false;
                          await loadManagerOptions(definition.kind);
                        }}
                      />{/key}
                  </div>{/if}
              {/if}
            {/if}
          {:else if definition.kind === 'seerr'}
            <p class="text-xs leading-6 text-muted">
              Discovery and requests are available on Home. Availability comes
              from Radarr and Sonarr.
            </p>
            {#if supportData?.initialized === false}<Notice tone="warning"
                >Seerr setup is still in progress.</Notice
              >{/if}
            <Button
              class="mt-3"
              variant="secondary"
              size="sm"
              disabled={busy}
              onclick={() =>
                void work(async () => {
                  await api('/admin/seerr/sync', 'POST');
                }, 'Service connections refreshed.')}
              >Refresh Radarr and Sonarr connections</Button
            >
          {:else if supportData}
            {#each supportData.health ?? [] as issue, index (index)}<Notice
                tone="warning"
              >
                {typeof issue === 'string' ? issue : issue.message}
              </Notice>{/each}
            {#if selectedKind === 'prowlarr'}
              {#if !supportData.indexers?.length}<p
                  class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
                >
                  No indexers configured.
                </p>{/if}
              {#each supportData.indexers ?? [] as indexer (indexer.id)}
                <div
                  class="service-row flex items-center justify-between gap-3 border-b border-line py-3 text-[11px] [&>div]:min-w-0"
                >
                  <div>
                    <strong>{indexer.name}</strong><span
                      class="row-status mt-1 block text-[10px] text-muted"
                      >{indexer.disabled_until
                        ? `Unavailable until ${indexer.disabled_until}`
                        : indexer.enabled
                          ? 'Enabled'
                          : 'Disabled'}</span
                    >
                  </div>
                  <div class="row-actions flex flex-wrap items-center gap-2">
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
              {#key connected.id}<IndexerOnboarding
                  serviceId={connected.id}
                  added={() => refreshSupport('prowlarr')}
                />{/key}
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
                <FormField
                  >Subtitle language<input
                    class={formControlClass}
                    bind:value={subtitle.language}
                    minlength="2"
                    maxlength="3"
                    placeholder="en"
                  /></FormField
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
                  class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
                >
                  No missing subtitles reported. Configure language profiles and
                  providers in Bazarr.
                </p>{/if}
              {#each [...(supportData.movies ?? []), ...(supportData.episodes ?? [])] as missing (`${missing.movie_id}:${missing.episode_id}`)}
                <div class="border-t border-line py-2">
                  <p
                    class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
                  >
                    {missing.title}
                  </p>
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
      {#if item && (live?.can_retire || (!live && ['blocked', 'retiring'].includes(item.state)))}<Notice
        >
          Retiring removes the installation and its API connection. Appdata,
          request history, and media files are kept.
        </Notice>{/if}
      <section
        class="work-section border-b border-line py-4 first-of-type:pt-0 last:border-b-0"
        aria-label="Updates"
      >
        <h3 class="mb-3.25 text-[12px] font-[650]">Updates</h3>
        {#if target}
          {#key target.id}<AutoSaveForm
              class="grid gap-3 [&_label]:m-0"
              label={`${definition.label} update settings`}
              value={{
                policy: servicePolicy.policy,
                window_start: servicePolicy.start,
                window_end: servicePolicy.end,
              }}
              onRevert={(previous) => {
                servicePolicy.policy = previous.policy;
                servicePolicy.start = previous.window_start;
                servicePolicy.end = previous.window_end;
              }}
              onsave={(submitted) =>
                api(
                  `/admin/service-updates/policy/${target.id}`,
                  'POST',
                  submitted,
                )}
            >
              {#snippet children(save)}
                <UpdatePolicyFields
                  bind:policy={servicePolicy.policy}
                  bind:start={servicePolicy.start}
                  bind:end={servicePolicy.end}
                  {timezone}
                  inherited={serverPolicy}
                  onChange={() => void save()}
                />
              {/snippet}
            </AutoSaveForm>{/key}
          {#if policy?.candidate && policy.candidate !== live?.image}<p
              class="candidate my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
            >
              Available image: <code>{policy.candidate}</code>
            </p>
          {:else if policy?.candidate && live?.image}<p
              class="my-2 text-xs text-muted"
            >
              Up to date.
            </p>
          {:else if !policy?.checked_at}<p class="my-2 text-xs text-muted">
              Waiting for the first automatic update check.
            </p>{/if}
          {#if policy?.checked_at}<p class="my-2 text-xs text-muted">
              Last check: {new Date(policy.checked_at * 1000).toLocaleString(
                undefined,
                { timeZone: timezone, hour12: timeFormat === '12h' },
              )}
            </p>{/if}
          {#if policy?.error}<Notice tone="warning" role="status">
              {policy.error}
            </Notice>{/if}
          {#if policy?.candidate && policy.candidate !== live?.image && !policy.error}
            <div class="row-actions flex flex-wrap items-center gap-2">
              <Button
                variant="secondary"
                size="form"
                disabled={busy}
                onclick={() => void work(() => preflight(selectedKind))}
                >Check compatibility</Button
              >
            </div>
          {/if}
          {#each serviceUpdatesList as update (update.id)}
            <Notice
              tone={[
                'blocked',
                'recovery-required',
                'runtime-failure',
                'rolled-back',
              ].includes(update.state)
                ? 'warning'
                : 'info'}
              role="status"
            >
              <strong
                >{update.classification === 'incompatible'
                  ? 'Update incompatible'
                  : update.classification === 'unable-to-verify'
                    ? 'Unable to verify update'
                    : (stages[update.state] ?? update.state)}</strong
              >
              {#if update.error}<p
                  class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
                >
                  {update.error}
                </p>{/if}
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
              {#if update.state === 'runtime-failure'}<p
                  class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
                >
                  Review the updated service before an explicit restore.
                </p>{/if}
            </Notice>
          {/each}
        {:else}<p
            class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
          >
            {connected
              ? 'Take ownership to manage updates here.'
              : 'Updates become available after managed setup.'}
          </p>{/if}
      </section>
    </div>
  </article>
  {#if feedback.general || approvalUsers.length}<footer
      class="services-footer mt-6 grid gap-5 border-t border-line pt-4.5 pl-4 compact:pl-3"
    >
      {#if feedback.general}<p
          class="my-2 wrap-anywhere text-[11px] leading-[1.6] text-muted"
          role="status"
        >
          {feedback.general}
        </p>{/if}
      {#if approvalUsers.length}<section
          aria-label="Automatic request approval"
        >
          <h3 class="mb-3.25 text-[12px] font-[650]">
            Automatic request approval
          </h3>
          <div class="row-actions flex flex-wrap items-center gap-2">
            {#each approvalUsers as user (user.id)}<Switch
                size="sm"
                checked={user.enabled}
                disabled={pendingActions.general}
                onCheckedChange={(enabled) =>
                  void work(
                    async () => {
                      await api(`/admin/acquisition/users/${user.id}`, 'PUT', {
                        enabled,
                      });
                      await loadApprovalUsers();
                    },
                    '',
                    'general',
                  )}>{user.username}</Switch
              >{/each}
          </div>
        </section>{/if}
    </footer>{/if}
</Panel>
