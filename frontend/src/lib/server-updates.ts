import { readable } from 'svelte/store';
import { api, ApiError, desktop, serverUrl } from './api';

export type ServerUpdate = {
  id: string;
  version: string;
  previous_version: string;
  stage: string;
  error: string | null;
  snapshot_ready: boolean;
  recovery_tested: boolean;
  activation_crossed: boolean;
  download?: {
    component: string;
    received: number;
    total: number | null;
  } | null;
};
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
  } | null;
  observation: { checked_at: number; error: string | null } | null;
  controller: { items: ServerUpdate[]; error?: string };
};
type Model = {
  status: ServerUpdateStatus | null;
  offlineSince: number;
  now: number;
  action: 'check' | 'install' | 'recover' | null;
  error: string;
};
let value: Model = {
  status: null,
  offlineSince: 0,
  now: Date.now(),
  action: null,
  error: '',
};
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
    value.status?.controller.items.some((item) =>
      [
        'preparing',
        'snapshotting',
        'validating',
        'preparing-activation',
        'isolated-migration',
        'creating-successor',
        'handoff',
        'activating',
        'recovering',
        'restoring-release',
      ].includes(item.stage),
    );
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
  void refreshServerUpdate();
  const refresh = () => void refreshServerUpdate();
  const timer = setInterval(refresh, 3000);
  window.addEventListener('thelxinoe-product-update', refresh);
  return () => {
    clearInterval(timer);
    window.removeEventListener('thelxinoe-product-update', refresh);
  };
});
export async function refreshServerUpdate() {
  if (loading) return;
  loading = true;
  const source = serverUrl();
  try {
    const status = await api<ServerUpdateStatus>('/admin/product-update');
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
      update({
        ...(acknowledged ? { error: '' } : {}),
        status,
        offlineSince: status.controller.error
          ? value.offlineSince || Date.now()
          : 0,
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
export async function serverUpdateAction(
  action: 'check' | 'install' | 'recover',
  operation?: string,
) {
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
  // Record before sending: losing the response during restart still reconnects.
  if (action === 'install' && !desktop)
    sessionStorage.setItem(reloadKey, version!);
  try {
    await api(
      '/admin/product-update/' +
        (action === 'recover' ? `${operation}/recover` : action),
      'POST',
      action === 'install'
        ? { version }
        : action === 'recover'
          ? { confirm: true }
          : {},
    );
    return true;
  } catch (e) {
    if (source === serverUrl()) {
      // Rejections are authoritative; transport/server failures may follow acceptance.
      if (!(e instanceof ApiError) || e.status >= 500)
        uncertainInstall = attempt;
      update({ error: e instanceof Error ? e.message : String(e) });
    }
    return false;
  } finally {
    await refreshServerUpdate();
    update({ action: null });
  }
}
