import { readable } from 'svelte/store';
import { api, ApiError, desktop, serverUrl } from './api';
import { startActivity } from './activity';
import { refreshAttention } from './attention';
import { inBackground } from './page-readiness';
import { RefreshTask } from './refresh-task';
import { formatTime } from './display.svelte';
import { onServerEvent } from './server-events.svelte';

export type ServerUpdate = {
  id: string;
  version: string;
  previous_version: string;
  stage: string;
  error: string | null;
  snapshot_ready: boolean;
  recovery_tested: boolean;
  activation_crossed: boolean;
  download?: Download | null;
  source_generation?: number | null;
  /** Why a blocked or recovered attempt stopped; older controllers omit it. */
  failure?:
    'environment' | 'interrupted' | 'validation' | 'incompatible' | null;
};
type Download = { component: string; received: number; total: number | null };
export type ServerUpdateStatus = {
  version: string;
  timezone: string;
  configured: boolean;
  policy: { policy: string; window_start: number; window_end: number };
  release: { version: string; notes: string } | null;
  request?: {
    id: string;
    version: string;
    previous_version: string;
    state: string;
    error: string | null;
    /** Why a queued request is waiting to retry, e.g. Docker is unavailable. */
    waiting?: string | null;
  } | null;
  observation: {
    checked_at: number;
    error: string | null;
    incompatible?: { version: string; reason: string } | null;
  } | null;
  automatic?: {
    version: string;
    failures: number;
    retry_at: number;
    error: string | null;
    held?: boolean;
  } | null;
  /** The failed attempt an administrator dismissed. */
  dismissed?: string | null;
  /** Images the controller downloads before preparing, while the server works. */
  download?: { version: string; download: Download | null } | null;
  controller: {
    items: ServerUpdate[];
    error?: string;
    generation?: number | null;
  };
};
type Action = 'check' | 'install' | 'recover' | 'cancel' | 'dismiss';
export type ServerUpdateModel = {
  status: ServerUpdateStatus | null;
  offlineSince: number;
  /** When the server last answered without its Docker controller. */
  controllerSince: number;
  /** Whether the controller last reported an operation in progress. */
  updating: boolean;
  now: number;
  action: Action | null;
  error: string;
};
type Model = ServerUpdateModel;
let value: Model = {
  status: null,
  offlineSince: 0,
  controllerSince: 0,
  updating: false,
  now: Date.now(),
  action: null,
  error: '',
};
const activation = [
  'preparing-activation',
  'isolated-migration',
  'creating-successor',
  'handoff',
  'activating',
];
// The server is fenced while these run; validation leaves it working.
const fenced = [
  'preparing',
  'snapshotting',
  ...activation,
  'recovering',
  'restoring-release',
];
const working = [...fenced, 'validating'];
/** A preparation for containers that no longer run, e.g. after `docker compose
 * up` installed a release. Older controllers do not retire these themselves. */
function obsoleteServerUpdate(
  item: ServerUpdate,
  controller: ServerUpdateStatus['controller'],
) {
  const moved =
    controller.generation != null &&
    item.source_generation !== controller.generation;
  return (
    item.stage === 'superseded' ||
    (moved &&
      (['blocked', 'ready'].includes(item.stage) ||
        (item.stage === 'recovered' && !item.snapshot_ready)))
  );
}
function newerVersion(version: string, than: string) {
  const [a, b] = [version, than].map((v) => v.split('.').map(Number));
  for (let i = 0; i < Math.max(a.length, b.length); i++)
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0);
  return false;
}
let publish: (value: Model) => void = () => {};
let loading = false;
let sourceServer = '';
let uncertainInstall: {
  source: string;
  version: string;
  previousId?: string;
  alreadyPending: boolean;
} | null = null;
function installAcknowledged(status: ServerUpdateStatus, source: string) {
  const attempt = uncertainInstall;
  const request = status.request;
  return Boolean(
    attempt &&
    request &&
    attempt.source === source &&
    request.version === attempt.version &&
    (request.id !== attempt.previousId || attempt.alreadyPending),
  );
}
export function isServerUpdateInterruption(error: unknown): boolean {
  if (error instanceof ApiError && error.code === 'maintenance') return true;
  if (
    sourceServer !== serverUrl() ||
    Date.now() - value.now > 120000 ||
    (value.offlineSince && Date.now() - value.offlineSince > 120000)
  )
    return false;
  const updating =
    value.status?.request?.state === 'pending' ||
    value.status?.controller.items.some((item) => working.includes(item.stage));
  // Background reads retain their cached data while Orbit handles reconnects.
  return (
    Boolean(updating) && (!(error instanceof ApiError) || error.status === 503)
  );
}
function update(next: Partial<Model>) {
  value = { ...value, ...next, now: Date.now() };
  publish(value);
}
export const serverUpdates = readable(value, (set) => {
  publish = set;
  // The desktop can switch servers without restarting the process.
  if (sourceServer !== serverUrl()) {
    sourceServer = serverUrl();
    uncertainInstall = null;
    update({ status: null, offlineSince: 0, error: '' });
  }
  const reads = new RefreshTask(refreshServerUpdate, 3000);
  reads.start();
  const stop = onServerEvent((event) => {
    if (['product.changed', 'server.reconnected'].includes(event.kind))
      void reads.refresh();
  });
  return () => {
    reads.stop();
    stop();
  };
});
export async function refreshServerUpdate() {
  if (loading) return;
  loading = true;
  const source = serverUrl();
  try {
    // The status waits on the Docker controller; its cards show their own loading.
    const status = await inBackground(() =>
      api<ServerUpdateStatus>('/admin/product-update'),
    );
    if (source === serverUrl()) {
      if (
        !desktop &&
        !value.action &&
        status.request?.state === 'failed' &&
        requestedWebVersion() === status.request.version
      )
        clearRequestedWebVersion();
      // A lost POST response is superseded by the durable matching intent.
      // Actual operation failures remain in request.error/controller state.
      const acknowledged = installAcknowledged(status, source);
      if (acknowledged) uncertainInstall = null;
      // A controller outage, e.g. while it is replaced, is not a server outage.
      const controllerDown = Boolean(status.controller.error);
      update({
        ...(acknowledged ? { error: '' } : {}),
        status,
        offlineSince: 0,
        controllerSince: controllerDown
          ? value.controllerSince || Date.now()
          : 0,
        updating: controllerDown
          ? value.updating
          : status.controller.items.some(
              (item) =>
                working.includes(item.stage) &&
                !obsoleteServerUpdate(item, status.controller),
            ),
      });
    }
  } catch {
    if (source === serverUrl())
      update({ offlineSince: value.offlineSince || Date.now() });
  } finally {
    loading = false;
  }
}

const reloadKey = 'thelxinoe:server-update';
export function requestedWebVersion() {
  return sessionStorage.getItem(reloadKey);
}
export function clearRequestedWebVersion() {
  sessionStorage.removeItem(reloadKey);
}
// Whether web tabs can load the version the server runs. An update the
// controller still runs may yet restore the previous version. Without
// controller state, or for a version it did not install, the server decides.
// Preparations never change the running version.
export function webUpdateSettled(status: ServerUpdateStatus, version: string) {
  if (status.version !== version) return false;
  if (status.controller.error) return true;
  const related = status.controller.items.filter(
    (item) =>
      ![
        'preparing',
        'snapshotting',
        'validating',
        'ready',
        'blocked',
        'superseded',
      ].includes(item.stage) &&
      !obsoleteServerUpdate(item, status.controller) &&
      (item.version === version || item.previous_version === version),
  );
  return (
    !related.length ||
    related.some(
      (item) =>
        (item.version === version && item.stage === 'committed') ||
        (item.previous_version === version &&
          ['restored', 'recovered'].includes(item.stage)),
    )
  );
}
/** `operation` is the journal a recovery or dismissal concerns. */
export async function serverUpdateAction(action: Action, operation?: string) {
  if (value.action) return;
  const version = value.status?.release?.version;
  if (action === 'install' && !version) return;
  const source = serverUrl();
  const attempt =
    action === 'install'
      ? {
          source,
          version: version!,
          previousId: value.status?.request?.id,
          alreadyPending: value.status?.request?.state === 'pending',
        }
      : null;
  uncertainInstall = null;
  update({ action, error: '' });
  const finish = startActivity({
    target: 'server',
    resource: null,
    message: {
      check: 'Checking server updates',
      recover: 'Recovering server update',
      install: 'Updating Thelxinoe server',
      cancel: 'Cancelling server update',
      dismiss: 'Dismissing server update',
    }[action],
  });
  // Record before sending: losing the response during restart still reconnects.
  if (action === 'install' && !desktop)
    sessionStorage.setItem(reloadKey, version!);
  const deadline = Date.now() + 120000;
  try {
    for (;;) {
      if (source !== serverUrl()) return false;
      try {
        await api(
          '/admin/product-update/' +
            (action === 'recover' ? `${operation}/recover` : action),
          'POST',
          action === 'install'
            ? { version }
            : action === 'recover'
              ? { confirm: true }
              : action === 'dismiss'
                ? { id: operation }
                : {},
        );
        return true;
      } catch (error) {
        // Only retry an explicit idle rejection: a lost response may follow acceptance.
        if (
          action !== 'recover' ||
          !(error instanceof ApiError) ||
          error.status !== 409 ||
          ![
            'Wait for current requests to finish and try again',
            'Wait for playback, downloads and background work to finish',
          ].includes(error.message) ||
          Date.now() >= deadline
        )
          throw error;
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
    }
  } catch (e) {
    if (source === serverUrl()) {
      // Rejections are authoritative; transport/server failures may follow acceptance.
      if (!(e instanceof ApiError) || e.status >= 500)
        uncertainInstall = attempt;
      update({ error: e instanceof Error ? e.message : String(e) });
    }
    return false;
  } finally {
    try {
      await refreshServerUpdate();
      await refreshAttention();
    } finally {
      update({ action: null });
      finish();
    }
  }
}

export type ServerUpdateView = {
  title: string;
  description: string;
  icon:
    | 'available'
    | 'download'
    | 'snapshot'
    | 'check'
    | 'refresh'
    | 'reconnect'
    | 'restore'
    | 'wait'
    | 'alert';
  tone: 'muted' | 'accent' | 'success' | 'warning' | 'danger';
  motion: 'none' | 'available' | 'working' | 'restoring';
  action: 'check' | 'install' | 'connection' | null;
  label?: string;
};
export type ServerUpdateCard = {
  /** The installed version: a new server answers before the controller accepts it. */
  version: string;
  view: ServerUpdateView;
  progress: number | null;
  /** A queued request that may still be withdrawn. */
  cancellable: boolean;
  /** The failed attempt an administrator may dismiss. */
  dismissible: string | null;
};
/** What the Product version card shows for the current update state. */
export function serverUpdateCard(
  model: ServerUpdateModel,
  installed = '—',
): ServerUpdateCard {
  const status = model.status;
  const request = status?.request;
  const pending = request?.state === 'pending';
  // As in the attention list, only the newest attempt speaks for past updates.
  const current = (item?: ServerUpdate) =>
    item && status && !obsoleteServerUpdate(item, status.controller)
      ? item
      : undefined;
  const operation = current(
    pending
      ? status?.controller.items.find((item) => item.id === request?.id)
      : status?.controller.items[0],
  );
  const stage = operation?.stage;
  const running = status?.version ?? installed;
  const version =
    operation &&
    activation.includes(operation.stage) &&
    operation.version === running
      ? operation.previous_version
      : pending && request?.version === running && stage !== 'committed'
        ? request.previous_version
        : running;
  const candidate = status?.release?.version;
  // Images download before the preflight; older controllers download while preparing.
  const fetched =
    status?.download &&
    (!operation || !working.includes(operation.stage)) &&
    status.download.version === (pending ? request?.version : candidate)
      ? status.download
      : null;
  const download =
    stage === 'preparing' ? operation?.download : fetched?.download;
  let dismissible: string | null = null;
  const busy = (
    title: string,
    description: string,
    icon: ServerUpdateView['icon'] = 'check',
    restoring = false,
  ): ServerUpdateView => ({
    title,
    // A refused action, e.g. a cancel after installation began, explains itself.
    description: [model.error, description].filter(Boolean).join('\n'),
    icon,
    tone: restoring || model.error ? 'warning' : 'accent',
    motion: restoring ? 'restoring' : 'working',
    action: null,
  });
  const attention = (
    title: string,
    description: string,
    label = 'Check server connection',
  ): ServerUpdateView => ({
    title,
    description,
    icon: 'alert',
    tone: 'danger',
    motion: 'none',
    action: 'connection',
    label,
  });
  const fetching = (download?: Download | null) =>
    busy(
      download
        ? `Downloading ${download.component === 'controller' ? 'controller' : 'server'} update`
        : 'Downloading the update',
      '',
      'download',
    );
  const waiting = () => {
    const newer =
      candidate && request && newerVersion(candidate, request.version)
        ? `\n${candidate} is also available; cancel to install it instead.`
        : '';
    return request?.waiting
      ? busy(
          'Waiting to retry the update',
          `${request.waiting}\nRetrying shortly.${newer}`,
          'wait',
        )
      : busy(
          'Waiting for the server to be idle',
          `Waiting for playback and background work.${newer}`,
          'wait',
        );
  };
  const view = ((): ServerUpdateView => {
    if (model.offlineSince) {
      if (model.now - model.offlineSince > 120000)
        return attention(
          'The server has not returned',
          'Still retrying. Click to check now.',
        );
      return busy(
        'Reconnecting to the server',
        'Updates continue in the background.',
        'reconnect',
      );
    }
    if (status?.controller.error) {
      // The server answers; its controller may be restarting into a new release.
      if (
        (pending || model.updating) &&
        model.now - model.controllerSince <= 120000
      )
        return busy(
          'Reconnecting to the update controller',
          'Updates continue in the background.',
          'reconnect',
        );
      return attention(
        'Controller unavailable',
        'Click to check again.',
        'Check update controller',
      );
    }
    if (model.action)
      return busy(
        {
          check: 'Checking for updates',
          recover: 'Requesting recovery',
          install: 'Queuing the update',
          cancel: 'Cancelling the update',
          dismiss: 'Dismissing the failed update',
        }[model.action],
        '',
      );
    switch (stage) {
      case 'preparing':
        return download ? fetching(download) : busy('Preparing the update', '');
      case 'snapshotting':
        return busy('Saving a recovery copy', '', 'snapshot');
      case 'validating':
        return busy('Checking the update', '');
      case 'ready':
        if (pending) return waiting();
        break;
      case 'preparing-activation':
      case 'isolated-migration':
      case 'creating-successor':
      case 'handoff':
      case 'activating':
        return busy(
          `Installing ${operation?.version}`,
          'Restarting and reconnecting.',
          'refresh',
        );
      case 'recovering':
      case 'restoring-release':
        return busy('Restoring the previous version', '', 'restore', true);
      case 'runtime-failure':
      case 'recovery-required':
        return attention(
          'The update needs attention',
          operation?.error ?? 'Check the connection or use Rollback.',
        );
    }
    if (fetched && (!pending || !operation)) return fetching(download);
    if (pending && !operation) return waiting();
    // A dismissed attempt, one for a version already running or one a restart
    // interrupted (it starts again) needs nothing.
    const relevant = (id: string, target: string) =>
      id !== status?.dismissed &&
      newerVersion(target, running) &&
      (!candidate || candidate === target);
    const failedRequest =
      request?.state === 'failed' &&
      (!operation || operation.id === request.id) &&
      relevant(request.id, request.version)
        ? request
        : null;
    const failedOperation =
      operation &&
      ['blocked', 'recovered', 'restored'].includes(operation.stage) &&
      operation.failure !== 'interrupted' &&
      relevant(operation.id, operation.version)
        ? operation
        : null;
    const automatic =
      status?.policy.policy === 'automatic' &&
      candidate &&
      status.automatic?.version === candidate &&
      (status.automatic.failures > 0 || status.automatic.held)
        ? status.automatic
        : null;
    const retry = automatic
      ? automatic.held
        ? `Automatic updates stopped trying ${candidate}.`
        : `Automatic updates retry after ${formatTime(automatic.retry_at, status?.timezone)}, within the maintenance window.`
      : '';
    const error = model.error || failedRequest?.error || '';
    if (error || failedOperation) {
      dismissible = failedOperation?.id ?? failedRequest?.id ?? null;
      const restored = failedOperation && failedOperation.stage !== 'blocked';
      return {
        title: restored
          ? `${version} has been restored`
          : 'Update could not complete',
        description: [
          error || failedOperation?.error || 'The previous version is running.',
          retry,
          candidate
            ? 'Click to retry the update.'
            : 'Click to check for updates.',
        ]
          .filter(Boolean)
          .join('\n'),
        icon: restored ? 'restore' : 'refresh',
        tone: 'warning',
        motion: 'none',
        action: candidate ? 'install' : 'check',
        label: candidate ? 'Retry server update' : 'Check for server updates',
      };
    }
    if (automatic)
      return {
        title: `Automatic update to ${candidate} failed`,
        description: [
          automatic.error ?? 'The last attempt did not complete.',
          retry,
          'Click to install it now.',
        ].join('\n'),
        icon: 'available',
        tone: 'warning',
        motion: 'none',
        action: 'install',
        label: `Update server to ${candidate}`,
      };
    if (candidate)
      return {
        title: `Update to ${candidate}`,
        description: 'Click to install and restart.',
        icon: 'available',
        tone: 'accent',
        motion: 'available',
        action: 'install',
        label: `Update server to ${candidate}`,
      };
    const incompatible = status?.observation?.incompatible;
    if (incompatible)
      return {
        title: `${incompatible.version} needs a manual update`,
        description: `${incompatible.reason}.\nFollow its release notes to update this server.`,
        icon: 'alert',
        tone: 'warning',
        motion: 'none',
        action: 'check',
        label: 'Check for server updates',
      };
    if (status?.observation?.error)
      return {
        title: 'Release check failed',
        description: `${status.observation.error}\nClick to check again.`,
        icon: 'refresh',
        tone: 'warning',
        motion: 'none',
        action: 'check',
        label: 'Check for server updates',
      };
    if (stage === 'committed' && operation?.version === status?.version)
      return {
        title: `Updated to ${version}`,
        description: 'Click to check for updates.',
        icon: 'refresh',
        tone: 'success',
        motion: 'none',
        action: 'check',
      };
    return {
      title: !status
        ? 'Checking update status'
        : !status.configured
          ? 'Release checking unavailable'
          : status.observation?.checked_at
            ? 'Up to date'
            : 'Waiting for the first release check',
      description: !status?.configured
        ? 'Release signing key unavailable.'
        : 'Click to check for updates.',
      icon: 'refresh',
      tone: 'muted',
      motion: 'none',
      action: status ? 'check' : null,
    };
  })();
  return {
    version,
    view,
    progress: download?.total
      ? (download.received / download.total) * 100
      : null,
    cancellable:
      pending &&
      !view.action &&
      !model.action &&
      !model.offlineSince &&
      !(operation && fenced.includes(operation.stage)),
    dismissible,
  };
}
