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
