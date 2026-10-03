import type { Provision, StackService, Update } from './feature';

export function isSetupActive(item: Provision | undefined) {
  return ['queued', 'installing', 'connecting', 'retiring'].includes(
    item?.state ?? '',
  );
}

export function activeServiceUpdate(updates: Update[]) {
  return updates
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
        'rollback-copying',
        'rollback-activating',
        'activating',
        'queued-recover',
      ].includes(entry.state),
    );
}

export function serviceStatus({
  item,
  live,
  connected,
  update,
  unavailable,
}: {
  item?: Provision;
  live?: StackService;
  connected?: { error?: string | null } | null;
  update?: Update;
  unavailable: boolean;
}) {
  if (live?.registered === false)
    return { label: 'Setup mismatch', tone: 'warn' };
  if (isSetupActive(item))
    return {
      label:
        item?.state === 'connecting'
          ? 'Connecting API'
          : item?.state === 'retiring'
            ? 'Retiring'
            : 'Installing',
      tone: 'busy',
    };
  if (item?.state === 'blocked') return { label: 'Setup blocked', tone: 'bad' };
  if (update?.state === 'activating')
    return { label: 'Connecting API', tone: 'busy' };
  if (live?.status === 'unavailable')
    return { label: 'Status unavailable', tone: 'warn' };
  if (live?.status === 'missing')
    return { label: 'Container missing', tone: 'bad' };
  if (live?.status === 'ready') return { label: 'Ready', tone: 'ok' };
  if (live?.running === false) return { label: 'Stopped', tone: 'muted' };
  if (connected?.error) return { label: 'API unavailable', tone: 'warn' };
  if (live?.running) return { label: 'Running', tone: 'ok' };
  if (connected) return { label: 'Connected', tone: 'ok' };
  if (unavailable) return { label: 'Status unavailable', tone: 'warn' };
  return { label: 'Not connected', tone: 'muted' };
}

export function hasNativeAccess(kind: string | null) {
  return [
    'radarr',
    'sonarr',
    'lidarr',
    'prowlarr',
    'bazarr',
    'nzbget',
  ].includes(kind ?? '');
}

export function hasServiceUrlBase(kind: string | null) {
  return hasNativeAccess(kind) && kind !== 'nzbget';
}

export type ContainerPort = {
  PrivatePort: number;
  PublicPort?: number;
  IP?: string;
  Type: string;
};
export type Container = {
  id: string;
  names: string[];
  image?: string;
  state?: string;
  ports?: ContainerPort[];
};

export function serviceUiUrl(
  explicit: string | undefined,
  ports: ContainerPort[],
  internalPort: number,
  serverOrigin: string,
) {
  if (explicit) {
    try {
      const url = new URL(explicit);
      if (
        ['http:', 'https:'].includes(url.protocol) &&
        !url.username &&
        !url.password
      )
        return url.href;
    } catch {
      /* An absent or invalid UI address has no link. */
    }
    return '';
  }
  const origin = new URL(serverOrigin);
  const localHost = ['localhost', '127.0.0.1', '[::1]'].includes(
    origin.hostname,
  );
  const port = ports.find(
    (entry) =>
      entry.Type === 'tcp' &&
      entry.PrivatePort === internalPort &&
      entry.PublicPort &&
      (localHost || !['127.0.0.1', '::1'].includes(entry.IP ?? '')),
  );
  if (!port?.PublicPort) return '';
  origin.protocol = 'http:';
  // Wildcard and loopback publications use the server host, never a Docker IP.
  if (port.IP && !['0.0.0.0', '::', '127.0.0.1', '::1'].includes(port.IP))
    origin.hostname = port.IP.includes(':') ? `[${port.IP}]` : port.IP;
  origin.port = String(port.PublicPort);
  origin.pathname = '/';
  origin.search = '';
  origin.hash = '';
  return origin.href;
}
