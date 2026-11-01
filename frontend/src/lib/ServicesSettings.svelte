<script lang="ts">
  import { revisions } from './server-events.svelte';
  import FormField from './ui/FormField.svelte';
  import { formatTimestamp } from './display.svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import UpdatePolicyFields from './ui/UpdatePolicyFields.svelte';
  import { formControlClass } from './ui/styles';
  import StatusIndicator from './ui/StatusIndicator.svelte';
  import Notice from './ui/Notice.svelte';
  import ProgressBar from './ui/ProgressBar.svelte';
  import { onMount, tick, untrack } from 'svelte';
  import AttentionDot from './ui/AttentionDot.svelte';
  import {
    attention,
    attentionDescription,
    refreshAttention,
  } from './attention';
  import { startActivity } from './activity';
  import { api, ApiError, desktop } from './api';
  import { RefreshTask } from './refresh-task';
  import { inBackground } from './page-readiness';
  import { followLink, routeHref, sectionRoute } from './navigation';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import ConfirmDialog from './providers/components/ui/ConfirmDialog.svelte';
  import Switch from './ui/Switch.svelte';
  import { Copy, LoaderCircle } from '@lucide/svelte';
  import OwnershipModal from './services/OwnershipModal.svelte';
  import NativeAccessSettings from './services/NativeAccessSettings.svelte';
  import ConnectionSettings from './services/ConnectionSettings.svelte';
  import RecyclarrSettings from './services/RecyclarrSettings.svelte';
  import RecyclarrSetup from './services/RecyclarrSetup.svelte';
  import DownloadsTable from './services/DownloadsTable.svelte';
  import NativeServiceLink from './services/NativeServiceLink.svelte';
  import ServiceUpdateRelease from './services/ServiceUpdateRelease.svelte';
  import {
    hasNativeAccess,
    activeServiceUpdate,
    isSetupActive,
    hasServiceUrlBase,
    type Container,
  } from './services/presentation';
  import { serviceIcons as icons } from './services/icons';
  import QualityProfileEditor from './services/QualityProfileEditor.svelte';
  import IndexerOnboarding from './services/IndexerOnboarding.svelte';
  import ConnectionTestButton from './ui/ConnectionTestButton.svelte';
  import ServiceConnections, {
    type ConnectionAction,
    type ServiceConnection,
  } from './ServiceConnections.svelte';

  let { selected, workflow, onSelect, openSettings } = $props<{
    selected?: string;
    workflow?: 'setup' | 'updates';
    onSelect?: (kind: string) => void;
    openSettings?: (name: string) => void;
  }>();

  import {
    definitions,
    stages,
    loaderSubsystems,
    type ServiceKind,
    type Definition,
    type ManagerDefaults,
    type ManagerOptions,
    type SupportSnapshot,
    type ServiceView,
    type ServicesView,
    type LoaderKey,
    type ServerUpdatePolicy,
    type SetupDraft,
    type InstallDraft,
    type DefaultsDraft,
    type UpdateDraft,
  } from './services/feature';
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
          urlBase: '',
        } satisfies SetupDraft,
      ]),
    ) as Record<ServiceKind, SetupDraft>,
  );
  const installs = $state(
    Object.fromEntries(
      definitions.map((definition) => [
        definition.kind,
        { hostPort: definition.hostPort } satisfies InstallDraft,
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
  // The guided ownership workflow; reopening resumes its durable operation.
  let ownershipModal = $state<{
    kind: ServiceKind;
    serviceId: string;
    transferId: string | null;
  } | null>(null);
  let forgetKind = $state<ServiceKind | null>(null);
  const snapshots = $state<Partial<Record<ServiceKind, SupportSnapshot>>>({});
  const managerOptions = $state<Partial<Record<ServiceKind, ManagerOptions>>>(
    {},
  );
  // A stopped container can be connected only after explicitly skipping verification.
  const unverifiedConnections = $state<Partial<Record<ServiceKind, string>>>(
    {},
  );

  let services = $state<ServiceView[]>([]),
    connections = $state<ServiceConnection[]>([]),
    containers = $state<Container[]>([]);
  let timezone = $state(''),
    serverPolicy = $state<ServerUpdatePolicy>({
      policy: 'automatic',
      window_start: 3,
      window_end: 5,
    });
  const pendingActions = $state<
    Partial<Record<ServiceKind | 'general', boolean>>
  >({});
  const pendingOperations = $state<Partial<Record<ServiceKind, boolean>>>({});
  const copyingCredentials = $state<
    Partial<Record<'username' | 'password', boolean>>
  >({});
  const feedback = $state<Partial<Record<ServiceKind | 'general', string>>>({});
  let removalKind = $state<ServiceKind | null>(null);
  // The parts the server could not read, and the read itself.
  let loadErrors = $state<Partial<Record<LoaderKey, string>>>({});

  function stageOf(kind: ServiceKind) {
    return services.find((item) => item.kind === kind);
  }
  function integration(definition: Definition) {
    return stageOf(definition.kind)?.integration ?? undefined;
  }
  // Only managers have request defaults.
  function manager(kind: ServiceKind) {
    const found = stageOf(kind)?.integration;
    return found && 'defaults' in found ? found : undefined;
  }
  function supportService(kind: ServiceKind) {
    const found = stageOf(kind)?.integration;
    return found && !('defaults' in found) ? found : undefined;
  }
  function provision(kind: ServiceKind) {
    return stageOf(kind)?.provision ?? undefined;
  }
  function runtime(kind: ServiceKind) {
    return stageOf(kind)?.runtime ?? undefined;
  }
  /** Owned, set up and not held: the only state with normal operations. */
  function operational(kind: ServiceKind) {
    const stage = stageOf(kind);
    return stage?.stage === 'owned' && !stage.hold;
  }
  function openOwnership(kind: ServiceKind) {
    const stage = stageOf(kind);
    const serviceId = stage?.connection?.id ?? stage?.ownership?.id;
    if (!serviceId) return;
    ownershipModal = {
      kind,
      serviceId,
      transferId:
        stage?.operation?.type === 'transfer'
          ? stage.operation.id
          : (stage?.draft_review ?? null),
    };
  }
  function updateTarget(kind: ServiceKind) {
    return stageOf(kind)?.update?.target;
  }
  function updatePolicy(kind: ServiceKind) {
    return stageOf(kind)?.update?.policy ?? undefined;
  }
  function serviceUpdates(kind: ServiceKind) {
    const latest = stageOf(kind)?.update?.latest;
    return latest ? [latest] : [];
  }
  let selectedKind = $state<ServiceKind>('seerr');
  $effect(() => {
    const kind = selected;
    if (kind && definitions.some((definition) => definition.kind === kind))
      untrack(() => selectService(kind as ServiceKind));
  });
  $effect(() => {
    const section = workflow;
    if (section)
      void tick().then(() =>
        document
          .querySelector(`[data-service-workflow="${section}"]`)
          ?.scrollIntoView({ block: 'start' }),
      );
  });
  const busy = $derived(isBusy(selectedKind));
  const detailErrors = $state<Partial<Record<ServiceKind, string>>>({});
  const detailLoading = $state<Partial<Record<ServiceKind, boolean>>>({});
  const definition = $derived(
    definitions.find((entry) => entry.kind === selectedKind)!,
  );
  const connected = $derived(integration(definition));
  const libraryError = $derived(
    connected && 'library_error' in connected ? connected.library_error : null,
  );
  const item = $derived(provision(selectedKind));
  const live = $derived(runtime(selectedKind));
  const selectedStage = $derived(stageOf(selectedKind));
  const selectedContainer = $derived(containerFor(definition));
  const selectedStatus = $derived(status(selectedKind));
  const nativeUrl = $derived(
    hasNativeAccess(selectedKind) ? connected?.access_url : null,
  );
  const progress = $derived(activity(selectedKind));
  const operating = $derived(serviceOperationActive(selectedKind));
  const supportData = $derived(snapshots[selectedKind]);
  const nzbgetLoginTarget = $derived(
    provision('nzbget') ?? supportService('nzbget'),
  );
  const target = $derived(updateTarget(selectedKind));
  const policy = $derived(updatePolicy(selectedKind));
  const servicePolicy = $derived(updateDrafts[selectedKind]);
  const serviceUpdatesList = $derived(serviceUpdates(selectedKind).slice(0, 1));

  function setupActive(kind: ServiceKind) {
    return isSetupActive(provision(kind));
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
    return activeServiceUpdate(serviceUpdates(kind));
  }
  function isBusy(kind: ServiceKind, retryRemoval = false) {
    return (
      !!pendingActions[kind] ||
      (!(retryRemoval && provision(kind)?.state === 'retiring') &&
        setupActive(kind)) ||
      !!activeUpdate(kind)
    );
  }
  function serviceOperationActive(kind: ServiceKind) {
    return (
      !!pendingOperations[kind] ||
      ['queued', 'installing', 'connecting'].includes(
        provision(kind)?.state ?? '',
      ) ||
      !!activeUpdate(kind)
    );
  }
  function canRetire(kind: ServiceKind) {
    const item = provision(kind);
    const live = runtime(kind);
    return (
      !serviceOperationActive(kind) &&
      !!item &&
      !item.hold?.startsWith('transfer') &&
      (live?.can_retire ||
        (!live && ['blocked', 'retiring'].includes(item.state)))
    );
  }
  function status(kind: ServiceKind) {
    return stageOf(kind)?.status ?? { label: 'Not connected', tone: 'muted' };
  }
  function containerFor(definition: Definition) {
    const id =
      provision(definition.kind)?.container_id ??
      integration(definition)?.container_id;
    return containers.find((container) => container.id === id);
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
      if (manager(kind) && runtime(kind)?.running !== false) {
        await loadManagerOptions(kind);
        selectedRead = kind;
      } else if (
        supportService(kind) &&
        (kind === 'prowlarr' || kind === 'nzbget')
      ) {
        await refreshSupport(kind);
        selectedRead = kind;
      }
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
  // A read that started before a newer one or a local change is dropped.
  let viewLoad = 0;
  async function loadView() {
    const request = ++viewLoad;
    try {
      const view = await api<ServicesView>('/admin/services');
      if (request !== viewLoad) return;
      services = view.items;
      containers = view.containers;
      connections = view.connections;
      timezone = view.timezone;
      serverPolicy = view.server_policy;
      loadErrors = view.errors;
      for (const definition of definitions) {
        const draft = defaults[definition.kind];
        if (draft && !manager(definition.kind)) draft.loaded = false;
      }
      syncUpdateDrafts();
    } catch (error) {
      if (request === viewLoad) loadErrors = { services: String(error) };
    }
  }
  async function testConnection(kind: ServiceKind) {
    const id = stageOf(kind)?.connection?.id;
    if (!id) return;
    await api(`/admin/services/${id}/test`, 'POST', {});
    await refresh();
  }
  async function forgetConnection(kind: ServiceKind) {
    const id = stageOf(kind)?.connection?.id;
    if (!id) return;
    await api(`/admin/services/${id}/forget`, 'POST', {});
    await refresh();
  }
  // The server announces each change to what this page shows; the slow poll
  // only covers a lost event stream.
  const reads = new RefreshTask(refreshData, 60_000);
  // The selected service itself is asked when it is chosen and after a change,
  // made here or reported by an event, not on every poll.
  let changed = true;
  let selectedRead: ServiceKind | undefined;
  const refresh = () => {
    changed = true;
    return reads.refresh();
  };
  async function refreshData() {
    const reread = changed;
    changed = false;
    await loadView();
    // The panel shows its own skeleton while it loads.
    if (reread || selectedRead !== selectedKind)
      void inBackground(() => loadSelectedData());
  }
  // NZBGet's queue moves on its own.
  const queueReads = new RefreshTask(
    () => (selectedKind === 'nzbget' ? loadSelectedData() : Promise.resolve()),
    4000,
  );
  let observedRevision: number | undefined;
  $effect(() => {
    const next = revisions.settings;
    untrack(() => {
      if (observedRevision !== undefined && observedRevision !== next)
        void refresh();
      observedRevision = next;
    });
  });
  async function connectionAction(
    connection: ServiceConnection,
    action: ConnectionAction,
  ) {
    const finish = startActivity({
      target: 'services',
      resource: connection.source_kind,
      message: `${action === 'disconnect' ? 'Disconnecting' : 'Connecting'} ${connection.target_name}`,
    });
    try {
      ++viewLoad;
      const updated = await api<ServiceConnection>(
        '/admin/service-connections',
        'POST',
        {
          source_id: connection.source_id,
          target_id: connection.target_id,
          action,
        },
      );
      ++viewLoad;
      connections = connections.map((item) =>
        item.id === updated.id ? updated : item,
      );
    } finally {
      await refreshAttention();
      finish();
    }
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
    const finish = startActivity({
      target: 'services',
      resource: kind === 'general' ? null : kind,
      message: `${kind === 'general' ? 'Media services' : (definitions.find((item) => item.kind === kind)?.label ?? kind)} action in progress`,
    });
    feedback[kind] = '';
    try {
      await action();
      if (success) feedback[kind] = success;
    } catch (error) {
      feedback[kind] = String(error);
    } finally {
      pendingActions[kind] = false;
      await refreshAttention();
      finish();
    }
  }
  async function connectExternal(definition: Definition, unverified = false) {
    const draft = setup[definition.kind];
    unverifiedConnections[definition.kind] = '';
    try {
      if (definition.role === 'manager') {
        await api('/admin/managers', 'POST', {
          name: draft.name,
          kind: definition.kind,
          container_id: draft.container,
          port: draft.port,
          api_key: draft.secret,
          url_base: draft.urlBase,
          allow_unverified: unverified,
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
          url_base: draft.urlBase,
          allow_unverified: unverified,
        });
      }
    } catch (error) {
      if (error instanceof ApiError && error.code === 'service_stopped') {
        unverifiedConnections[definition.kind] = error.message;
        return;
      }
      throw error;
    }
    draft.secret = '';
    await refresh();
    feedback[definition.kind] = unverified
      ? `${definition.label} connected without verification. Start its container, then test it from Edit connection.`
      : `${definition.label} connected.`;
  }
  async function operate(
    kind: ServiceKind,
    action: () => Promise<void>,
    success = '',
  ) {
    await work(
      async () => {
        pendingOperations[kind] = true;
        try {
          await action();
        } finally {
          delete pendingOperations[kind];
        }
      },
      success,
      kind,
    );
  }
  async function installManaged(definition: Definition) {
    const draft = installs[definition.kind];
    await api('/admin/stack/install', 'POST', {
      kind: definition.kind,
      host_port: draft.hostPort,
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
    draft.root_folder = service.defaults.root_folder ?? '';
    draft.quality_profile = service.defaults.quality_profile ?? 0;
    draft.metadata_profile = service.defaults.metadata_profile ?? null;
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
  async function stackAction(kind: ServiceKind, action: string) {
    const service = runtime(kind) ?? provision(kind);
    if (!service) return;
    await api(`/admin/stack/${service.id}/action`, 'POST', { action });
    if (action === 'remove') {
      const draft = defaults[kind];
      if (draft) draft.loaded = false;
      delete managerOptions[kind];
      delete snapshots[kind];
    }
    await refresh();
  }
  async function retryProvision(kind: ServiceKind) {
    const item = provision(kind);
    if (!item) return;
    await api(`/admin/stack/${item.id}/retry`, 'POST', {});
    await refresh();
  }

  async function preflight(kind: ServiceKind) {
    const target = updateTarget(kind);
    if (!target) return;
    await api(`/admin/service-updates/preflight/${target.id}`, 'POST', {});
    await loadView();
  }
  async function updateAction(id: string, action: 'activate' | 'recover') {
    await api(`/admin/service-updates/${id}/${action}`, 'POST', {});
    await loadView();
  }

  onMount(() => {
    const stops = [reads.start(), queueReads.start(false)];
    return () => stops.forEach((stop) => stop());
  });
</script>

<section
  aria-label="Media services"
  class="services-panel col-span-full grid min-w-0 gap-5"
>
  <ConfirmDialog
    open={removalKind !== null}
    title={`Remove ${definitions.find((service) => service.kind === removalKind)?.label ?? 'service'}?`}
    message="Delete this service's container, configuration, and retained update copies. Its connections in other services will be removed automatically. Media files and other services are kept."
    confirmLabel="Remove service and configuration"
    eyebrow="REMOVE / MEDIA SERVICE"
    danger
    onCancel={() => (removalKind = null)}
    onConfirm={() => {
      const kind = removalKind;
      removalKind = null;
      if (kind) void work(() => stackAction(kind, 'remove'), '', kind, true);
    }}
  />
  <ConfirmDialog
    open={forgetKind !== null}
    title={`Forget ${definitions.find((service) => service.kind === forgetKind)?.label ?? 'service'}?`}
    message="Thelxinoe deletes only its own connection record. The container, its configuration and its data are not touched."
    confirmLabel="Forget connection"
    eyebrow="FORGET / CONNECTED SERVICE"
    onCancel={() => (forgetKind = null)}
    onConfirm={() => {
      const kind = forgetKind;
      forgetKind = null;
      if (kind) void work(() => forgetConnection(kind), '', kind, true);
    }}
  />
  {#if ownershipModal}
    {@const modal = ownershipModal}
    <OwnershipModal
      serviceId={modal.serviceId}
      transferId={modal.transferId}
      label={definitions.find((service) => service.kind === modal.kind)
        ?.label ?? modal.kind}
      onClose={() => (ownershipModal = null)}
      changed={refresh}
      openService={() => selectService(modal.kind)}
    />
  {/if}
  <!-- One tab per service; the strip keeps every status visible at a glance.
       Narrow containers stack each tab so all eight fit in two rows. -->
  <nav
    class="service-strip grid min-w-0 grid-cols-2 gap-px border border-line bg-line @min-[270px]/settings:grid-cols-4 @min-[900px]/settings:grid-cols-8"
    aria-label="Select service"
  >
    {#each definitions as service (service.kind)}
      {@const state = status(service.kind)}
      <a
        href={`#admin/services/${service.kind}`}
        class="service-tab relative flex min-w-0 cursor-pointer flex-col items-center justify-center gap-1.5 bg-surface px-1 py-3 text-center text-foreground hover:bg-surface-soft aria-[current=true]:bg-accent-soft aria-[current=true]:text-accent aria-[current=true]:shadow-[inset_0_-2px_0_var(--accent)] @min-[480px]/settings:flex-row @min-[480px]/settings:justify-start @min-[480px]/settings:gap-2.5 @min-[480px]/settings:px-3 @min-[480px]/settings:py-2.5 @min-[480px]/settings:text-left [&>img]:size-6 [&>img]:shrink-0 [&>img]:object-contain [&_strong]:block [&_strong]:truncate [&_strong]:text-xs [&_strong]:font-[650]"
        aria-current={selectedKind === service.kind ? 'true' : undefined}
        aria-label={service.label}
        aria-describedby={`service-attention-${service.kind}`}
        onclick={(event) => {
          if (!followLink(event)) return;
          event.preventDefault();
          if (onSelect) onSelect(service.kind);
          else selectService(service.kind);
        }}
      >
        <img src={icons[service.kind]} alt="" />
        <span class="max-w-full min-w-0 @min-[480px]/settings:flex-1"
          ><strong>{service.label}</strong><StatusIndicator
            class="service-status mt-0.5 justify-center @min-[480px]/settings:justify-start"
            tone={state.tone}>{state.label}</StatusIndicator
          ></span
        >
        <AttentionDot
          items={$attention.filter(
            (item) =>
              item.target === 'services' && item.resource === service.kind,
          )}
        />
        <span id={`service-attention-${service.kind}`} class="sr-only"
          >{attentionDescription(
            $attention.filter(
              (item) =>
                item.target === 'services' && item.resource === service.kind,
            ),
          )}</span
        >
      </a>
    {/each}
  </nav>
  {#each loaderSubsystems as subsystem (subsystem.key)}
    {#if loadErrors[subsystem.key]}
      <Notice
        tone="danger"
        role="alert"
        class="mb-0"
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
    class="service-detail grid min-w-0 gap-5"
    data-page-region="service"
    aria-label={`${definition.label} service`}
  >
    <!-- Identity, status, actions and facts for the selected service. -->
    <aside
      class="service-rail panel settings-card min-w-0 border border-line bg-surface"
      aria-label="Service controls"
    >
      {#key selectedKind}
        <div
          class="rail-identity flex min-w-0 flex-col gap-3 px-5.5 py-4 compact:px-4"
        >
          <div
            class="rail-heading flex items-center gap-3.5 [&_img]:size-11 [&_img]:shrink-0 [&_img]:object-contain"
          >
            <img src={icons[definition.kind]} alt="" />
            <div class="min-w-0">
              <h2 class="m-0 text-lg font-semibold tracking-[-0.01em]">
                {definition.label}
              </h2>
              <span class="block text-xs text-muted"
                >{definition.description}</span
              >
            </div>
            <StatusIndicator
              class="service-status ml-auto shrink-0 text-[11px]"
              tone={selectedStatus.tone}>{selectedStatus.label}</StatusIndicator
            >
          </div>
          {#if nativeUrl && connected}<div
              class="service-url-line flex min-h-4.5 min-w-0 items-center gap-1"
            >
              <NativeServiceLink
                id={connected.id}
                label={definition.label}
                path={nativeUrl}
              />
              {#if definition.kind === 'nzbget' && nzbgetLoginTarget && !operating}
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
          <div class="rail-progress text-xs text-muted empty:hidden">
            {#if progress}<span>{progress}</span>
              <ProgressBar
                class="activity-bar mt-1.75 h-0.75"
                indeterminate
                label={`${definition.label}: ${progress}`}
              ></ProgressBar>{:else if feedback[definition.kind]}<span
                class="action-feedback block truncate"
                role="status"
                title={feedback[definition.kind]}
                >{feedback[definition.kind]}</span
              >{/if}
          </div>
        </div>
        <div
          class="rail-actions flex min-w-0 flex-wrap items-center gap-2 border-t border-line px-5.5 py-3 compact:px-4 [&_button]:whitespace-normal"
        >
          {#if selectedStage?.operation?.type === 'transfer'}
            <!-- The operation stays reachable while its modal is closed. -->
            <Button size="form" onclick={() => openOwnership(definition.kind)}
              >{['failed', 'suspended'].includes(selectedStage.operation.state)
                ? 'Open transfer recovery'
                : 'Open ownership transfer'}</Button
            >
          {:else if !operating}
            {#if selectedStage?.stage === 'connected'}
              <Button
                size="form"
                disabled={isBusy(definition.kind)}
                onclick={() => openOwnership(definition.kind)}
                >{selectedStage.draft_review
                  ? 'Continue ownership review'
                  : 'Take ownership'}</Button
              >
              <Button
                size="form"
                variant="secondary"
                disabled={isBusy(definition.kind)}
                onclick={() =>
                  void work(
                    () => testConnection(definition.kind),
                    `${definition.label} connection checked.`,
                  )}>Test connection</Button
              >
              <Button
                size="form"
                variant="ghost"
                disabled={isBusy(definition.kind)}
                onclick={() => {
                  feedback[definition.kind] = '';
                  forgetKind = definition.kind;
                }}>Forget connection</Button
              >
            {:else if item?.state === 'blocked'}
              <Button
                size="form"
                disabled={isBusy(definition.kind)}
                onclick={() =>
                  void operate(definition.kind, () =>
                    retryProvision(definition.kind),
                  )}>Retry setup</Button
              >
            {:else if live && live.registered !== false && !setupActive(definition.kind) && operational(definition.kind)}
              {#if live.existence === 'present'}
                {#each definition.role === 'job' ? ['reconcile'] : live.running ? ['restart', 'stop', 'reconcile'] : ['start', 'reconcile'] as action (action)}
                  <Button
                    size="form"
                    variant="secondary"
                    disabled={!canControl(definition.kind, action)}
                    onclick={() =>
                      void work(() => stackAction(definition.kind, action))}
                    >{action === 'reconcile'
                      ? 'Repair configuration'
                      : action[0].toUpperCase() + action.slice(1)}</Button
                  >
                {/each}
              {/if}
              {#if live.can_recreate}<Button
                  size="form"
                  disabled={isBusy(definition.kind)}
                  onclick={() =>
                    void operate(definition.kind, () =>
                      stackAction(definition.kind, 'recreate'),
                    )}>Recreate and start</Button
                >{/if}
            {/if}
            {#if canRetire(definition.kind)}
              <Button
                size="form"
                variant="danger"
                class="ml-auto"
                disabled={isBusy(definition.kind) ||
                  setupActive(definition.kind)}
                onclick={() =>
                  void work(() => stackAction(definition.kind, 'retire'))}
                >Retire and keep data</Button
              >
            {/if}
            {#if item && !item.hold?.startsWith('transfer') && (live?.can_remove || (item.state === 'retiring' && item.operation === 'remove'))}
              <Button
                size="form"
                variant="danger"
                class={canRetire(definition.kind) ? '' : 'ml-auto'}
                disabled={isBusy(definition.kind, true)}
                onclick={() => {
                  feedback[definition.kind] = '';
                  removalKind = definition.kind;
                }}>Remove service</Button
              >
            {/if}
            {#if canRetire(definition.kind)}
              <p class="m-0 basis-full text-xs text-muted">
                Retiring removes the installation and its API connection.
                Appdata, request history, and media files are kept.
              </p>
            {/if}
            {#if !selectedStage || selectedStage.stage === 'none'}<span
                class="text-xs text-muted">Choose how to connect below.</span
              >{/if}
          {/if}
        </div>
        <dl
          class="rail-meta m-0 grid min-w-0 grid-cols-4 gap-x-4 gap-y-3 border-t border-line bg-surface-soft px-5.5 py-3 compact:grid-cols-2 compact:px-4"
        >
          <div>
            <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
              Ownership
            </dt>
            <dd class="mt-0.75 wrap-anywhere text-[11px]">
              {selectedStage?.stage === 'owned'
                ? `Owned${selectedStage.ownership?.origin === 'transferred' ? ' · transferred' : ''}${selectedStage.hold ? ' · held' : ''}`
                : selectedStage?.stage === 'connected'
                  ? 'Connected · external'
                  : 'Not configured'}
            </dd>
          </div>
          <div>
            <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
              Application version
            </dt>
            <dd class="mt-0.75 wrap-anywhere text-[11px]">
              {connected?.version || selectedStage?.connection?.version || '—'}
            </dd>
          </div>
          <div>
            <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
              Container
            </dt>
            <dd class="mt-0.75 wrap-anywhere text-[11px]">
              {live?.name ||
                selectedContainer?.names[0]?.replace(/^\//, '') ||
                selectedStage?.connection?.container_name ||
                (connected ? 'Unavailable' : '—')}
            </dd>
          </div>
          <div>
            <dt class="text-[9px] tracking-[0.07em] text-muted uppercase">
              Installed image
            </dt>
            <dd class="image-value mt-0.75 wrap-anywhere text-[10px]">
              {live?.image || selectedContainer?.image || '—'}
            </dd>
          </div>
        </dl>
      {/key}
    </aside>
    <div class="service-workspace contents [&_form>button]:w-fit">
      {#if live?.registered === false}
        <div
          class="col-span-full border-l-2 border-warning bg-surface px-3 py-2 text-xs leading-5"
          role="status"
          aria-label="Service setup mismatch"
        >
          Saved service data has no matching record in this server profile.
          Restore the matching server profile to manage it. For a new instance,
          use fresh storage for both the server and its controller.
        </div>
      {/if}
      {#if item?.error || libraryError || live?.inspection_error || live?.error || live?.drift}
        <Notice class="col-span-full mb-0" tone="warning" role="status">
          {#each [...new Set([item?.error, libraryError, live?.inspection_error, live?.error].filter(Boolean))] as error (error)}<p
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
      {#if selectedStage?.stage === 'connected'}
        <section
          class="work-section settings-section"
          aria-label="Connected service"
        >
          <h3 class="mb-3.25 text-[12px] font-[650]">Connected</h3>
          <p class="mb-3 text-[11px] leading-[1.6] text-muted">
            Thelxinoe only checks this externally managed {definition.label}'s
            health and identity. Requests, downloads, library sync, links and
            container management become available after you take ownership.
          </p>
          <dl
            class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-[11px]"
          >
            <dt class="text-muted">Status</dt>
            <dd>
              {selectedStage.connection?.verified &&
              !selectedStage.connection.error
                ? 'Verified'
                : 'Unverified'}
            </dd>
            <dt class="text-muted">Version</dt>
            <dd>{selectedStage.connection?.version || '—'}</dd>
            <dt class="text-muted">Container</dt>
            <dd class="wrap-anywhere">
              {selectedStage.connection?.container_name}
              ({selectedStage.connection?.container_id.slice(0, 12)})
            </dd>
            <dt class="text-muted">Docker engine</dt>
            <dd class="wrap-anywhere">
              {selectedStage.connection?.engine || '—'}
            </dd>
            <dt class="text-muted">Last check</dt>
            <dd>
              {selectedStage.connection?.checked_at
                ? formatTimestamp(selectedStage.connection.checked_at)
                : 'Not checked yet'}{#if selectedStage.connection?.error}<span
                  class="block text-warning"
                  >{selectedStage.connection.error}</span
                >{/if}
            </dd>
          </dl>
        </section>
      {:else if selectedStage?.stage === 'owned' && selectedStage.hold && selectedStage.hold !== 'setup'}
        <Notice class="col-span-full mb-0" tone="warning" role="status">
          <strong>Owned, under an operational hold</strong>
          <p>
            Normal operations stay paused until its ownership transfer finishes
            its required checks or a recovery completes.
          </p>
        </Notice>
      {/if}
      {#if !selectedStage || selectedStage.stage === 'none'}
        <section
          class="work-section settings-section"
          aria-label="Service setup"
          data-service-workflow="setup"
        >
          <h3 class="mb-3.25 text-[12px] font-[650]">
            Set up {definition.label}
          </h3>
          <div class="setup-forms grid gap-5.5">
            {#if definition.role === 'job'}<RecyclarrSetup
                {containers}
                install={async () => {
                  await installManaged(definition);
                }}
                changed={refresh}
              />{:else}
              <form
                class="grid gap-3 [&_label]:m-0"
                aria-label={`Install managed ${definition.label}`}
                onsubmit={(event) => {
                  event.preventDefault();
                  void operate(definition.kind, () =>
                    installManaged(definition),
                  );
                }}
              >
                <strong class="text-xs">Install and own it</strong>
                {#if definition.internalPort !== null}<div
                    class="grid grid-cols-2 gap-3 compact:grid-cols-1"
                  >
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
                  </div>{/if}
                <Button
                  type="submit"
                  variant="secondary"
                  size="form"
                  disabled={busy || Boolean(loadErrors.stack)}
                  >Install {definition.label}</Button
                >
              </form>{/if}
            {#if definition.role !== 'job'}<form
                class="grid gap-3 border-t border-line pt-4 [&_label]:m-0"
                aria-label={`Connect existing ${definition.label}`}
                onsubmit={(event) => {
                  event.preventDefault();
                  void work(() => connectExternal(definition));
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
                    required={definition.kind !== 'nzbget'}
                    autocomplete="new-password"
                  /></FormField
                >
                {#if hasServiceUrlBase(definition.kind)}
                  <FormField
                    >Existing URL Base<input
                      class={formControlClass}
                      bind:value={setup[definition.kind].urlBase}
                      placeholder="Leave empty if the service has no URL Base"
                      maxlength="160"
                    /></FormField
                  >
                {/if}
                <Button
                  type="submit"
                  size="form"
                  disabled={busy || !containers.length}
                  >Connect {definition.label}</Button
                >
                {#if unverifiedConnections[definition.kind]}
                  <Notice
                    tone="warning"
                    role="status"
                    aria-label={`${definition.label} container stopped`}
                  >
                    <p>{unverifiedConnections[definition.kind]}</p>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={busy}
                      onclick={() =>
                        void work(() => connectExternal(definition, true))}
                      >Continue without checking</Button
                    >
                  </Notice>
                {/if}
              </form>{/if}
          </div>
        </section>
      {/if}
      {#if connected && !setupActive(selectedKind) && (selectedStage?.stage === 'connected' || operational(selectedKind)) && !selectedStage?.operation}
        {#key connected.id}
          <ConnectionSettings
            service={connected}
            label={definition.label}
            disabled={busy}
            containers={provision(definition.kind) ? undefined : containers}
            changed={refresh}
          />
        {/key}
        {#if hasNativeAccess(selectedKind) && operational(selectedKind) && (!connected.access_url || connected.native_url)}
          <NativeAccessSettings
            id={connected.id}
            url={connected.native_url ?? ''}
            changed={refresh}
          />
        {/if}

        {#if operational(selectedKind) && definition.role !== 'job'}
          <div class="grid min-w-0 gap-5">
            {#if definition.role === 'manager' || definition.kind === 'prowlarr' || definition.kind === 'nzbget'}
              <section
                class="work-section settings-section"
                aria-label={definition.role === 'manager'
                  ? 'Acquisition defaults'
                  : definition.kind === 'prowlarr'
                    ? 'Indexers'
                    : 'Downloads'}
              >
                <div
                  class="work-head mt-0 mb-4 flex items-center justify-between gap-3 compact:flex-wrap [&_h3]:m-0"
                >
                  <h3 class="mb-3.25 text-[12px] font-[650]">
                    {definition.role === 'manager'
                      ? 'Acquisition defaults'
                      : definition.kind === 'prowlarr'
                        ? 'Indexers'
                        : 'Downloads'}
                  </h3>
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
                        )}
                      >{supportData.paused ? 'Resume all' : 'Pause all'}</Button
                    >{/if}
                </div>
                {#if detailErrors[selectedKind]}<Notice
                    tone="danger"
                    role="alert"
                  >
                    {detailErrors[selectedKind]}
                  </Notice>{/if}
                {#if detailLoading[selectedKind] && !supportData && !defaults[selectedKind]?.loaded}<ContentSkeleton
                    label="Loading service settings"
                    variant="settings"
                  />{/if}
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
                          <FormField
                            >Root folder<select
                              class={formControlClass}
                              bind:value={draft.root_folder}
                              required
                            >
                              <option value="" disabled
                                >Choose an existing root folder</option
                              >
                              {#each options.roots as root (root.id)}<option
                                  value={root.path}>{root.path}</option
                                >{/each}
                            </select></FormField
                          >
                          <FormField
                            >{definition.kind === 'lidarr'
                              ? 'Quality profile'
                              : 'Default request profile'}<select
                              class={formControlClass}
                              value={draft.quality_profile || ''}
                              onchange={(event) =>
                                (draft.quality_profile = Number(
                                  event.currentTarget.value,
                                ))}
                              required
                              ><option value="" disabled
                                >Choose an existing profile</option
                              >{#each options.profiles as option (option.id)}<option
                                  value={option.id}>{option.name}</option
                                >{/each}</select
                            ></FormField
                          >
                          {#if definition.kind === 'lidarr'}<FormField
                              >Metadata profile<select
                                class={formControlClass}
                                value={draft.metadata_profile ?? ''}
                                onchange={(event) =>
                                  (draft.metadata_profile =
                                    Number(event.currentTarget.value) || null)}
                                required
                                ><option value="" disabled
                                  >Choose an existing metadata profile</option
                                >{#each options.metadata_profiles as option (option.id)}<option
                                    value={option.id}>{option.name}</option
                                  >{/each}</select
                              ></FormField
                            >{/if}
                          <Switch bind:checked={draft.monitored} size="sm"
                            >Monitor and search requests</Switch
                          >
                        </AutoSaveForm>{/key}
                      {#if definition.kind === 'radarr' || definition.kind === 'sonarr'}
                        {@const selectedProfile = options.profiles.find(
                          (profile) => profile.id === draft.quality_profile,
                        )}
                        <div
                          class="mt-3 flex flex-wrap gap-x-4 gap-y-2 text-[11px]"
                        >
                          {#if connected.access_url && !desktop}<a
                              class="text-foreground underline decoration-accent underline-offset-4"
                              href={`${connected.access_url.replace(/\/$/, '')}/settings/profiles`}
                              target="_blank"
                              rel="noopener noreferrer"
                              >Manage profiles in {definition.label}</a
                            >{/if}
                          {#if selectedProfile?.url?.startsWith('https://')}<a
                              class="text-foreground underline decoration-accent underline-offset-4"
                              href={selectedProfile.url}
                              target="_blank"
                              rel="noopener noreferrer">View TRaSH profile</a
                            >{/if}
                        </div>
                      {/if}
                      <Switch
                        size="sm"
                        class="mt-3"
                        checked={manager(selectedKind)?.retention_enabled ??
                          false}
                        disabled={busy}
                        onCheckedChange={(enabled) =>
                          void work(async () => {
                            await api(
                              `/admin/managers/${connected.id}/retention`,
                              'PUT',
                              { enabled },
                            );
                            await loadView();
                          })}>Allow auto-delete for this service</Switch
                      >
                      {#if definition.kind === 'lidarr'}<div class="mt-4">
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
                        <div
                          class="row-actions flex flex-wrap items-center gap-2"
                        >
                          <ConnectionTestButton
                            size="sm"
                            label="Test"
                            disabled={busy}
                            test={async () => {
                              feedback.prowlarr = '';
                              await supportCommand('prowlarr', 'test', {
                                item_id: indexer.id,
                              });
                            }}
                            onError={(error) =>
                              (feedback.prowlarr =
                                error instanceof Error
                                  ? error.message
                                  : String(error))}
                          />
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
                  {/if}
                {/if}
              </section>
            {/if}
            {#if !loadErrors.connections}<ServiceConnections
                kind={selectedKind}
                {connections}
                {busy}
                onaction={connectionAction}
              />{/if}
          </div>{/if}
      {/if}
      {#if selectedKind === 'recyclarr' && item?.state === 'complete' && operational('recyclarr')}<RecyclarrSettings
        />{/if}
      {#if operational(selectedKind) && (connected || (definition.role === 'job' && live)) && !setupActive(selectedKind)}
        <section
          class="work-section settings-section"
          aria-label="Updates"
          data-service-workflow="updates"
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
            {#if policy?.candidate && policy.candidate !== live?.image}
              <ServiceUpdateRelease
                image={policy.candidate}
                installedVersion={connected?.version}
                release={policy.release}
              />
            {:else if policy?.candidate && live?.image}<p
                class="my-2 text-xs text-muted"
              >
                Up to date.
              </p>
            {:else if !policy?.checked_at}<p class="my-2 text-xs text-muted">
                Waiting for the first automatic update check.
              </p>{/if}
            {#if policy?.checked_at}<p class="my-2 text-xs text-muted">
                Last check: {formatTimestamp(
                  policy.checked_at,
                  timezone || undefined,
                )}
              </p>{/if}
            {#if policy?.error}<Notice
                class="col-span-full"
                tone="warning"
                role="status"
              >
                {policy.error}
              </Notice>{/if}
            {#if policy?.candidate && policy.candidate !== live?.image && !policy.error}
              <div class="row-actions flex flex-wrap items-center gap-2">
                <Button
                  variant="secondary"
                  size="form"
                  disabled={busy}
                  onclick={() =>
                    void operate(selectedKind, () => preflight(selectedKind))}
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
                      void operate(selectedKind, () =>
                        updateAction(update.id, 'activate'),
                      )}>Install verified update</Button
                  >{/if}
                <!-- A blocked check normally restored its service; one left updating needs recovery. -->
                {#if ['recovery-required', 'rollback-activating'].includes(update.state) || (update.state === 'blocked' && live?.phase === 'updating')}<Button
                    variant="secondary"
                    size="sm"
                    disabled={busy}
                    onclick={() =>
                      void operate(selectedKind, () =>
                        updateAction(update.id, 'recover'),
                      )}>Recover before activation</Button
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
      {/if}
      {#if selectedKind === 'seerr' && operational('seerr')}
        <!-- Per-person approval lives with the people it concerns. -->
        <p
          class="col-span-full m-0 text-xs text-muted"
          data-service-note="approval"
        >
          Automatic request approval is set per person under
          <a
            class="text-foreground underline decoration-accent underline-offset-4"
            href={routeHref(sectionRoute('Settings', 'people'))}
            onclick={(event) => {
              if (!openSettings || !followLink(event)) return;
              event.preventDefault();
              openSettings('people');
            }}>People</a
          >.
        </p>
      {/if}
    </div>
  </article>
  {#if feedback.general}<p
      class="my-0 wrap-anywhere text-[11px] leading-[1.6] text-muted"
      role="status"
    >
      {feedback.general}
    </p>{/if}
</section>
