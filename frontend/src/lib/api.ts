import { invoke, isTauri } from '@tauri-apps/api/core';
import { version as clientVersion } from '../../package.json';
import { captureSession, invalidateSession } from './session';
import { trackPageWork } from './page-readiness';
export const desktop = typeof window !== 'undefined' && isTauri();
// The API this build speaks; servers report the range of client APIs they accept.
export const clientApi = __THELXINOE_API_VERSION__;
let desktopServer = '';
export async function initializeTransport() {
  if (desktop) desktopServer = await invoke<string>('server_url');
}
export async function changeServer(value: string) {
  invalidateSession();
  if (desktop) desktopServer = await invoke<string>('change_server', { value });
}
export type User = {
  id: string;
  username: string;
  role: 'admin' | 'user';
  timezone: string;
  avatar?: string | null;
};
export type Job = {
  id: string;
  kind: string;
  state: 'queued' | 'running' | 'complete' | 'failed';
  error: string | null;
};
export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
    if (status === 426 && typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('thelxinoe-update-required', { detail: message }),
      );
    }
  }
  // Server messages are written for people; never prefix them with "Error:".
  override toString() {
    return this.message;
  }
}
export function serverUrl(): string {
  return desktop ? desktopServer : '';
}
export function api<T>(
  path: string,
  method = 'GET',
  body?: unknown,
): Promise<T> {
  return trackPageWork(() => request<T>(path, method, body));
}
async function request<T>(
  path: string,
  method = 'GET',
  body?: unknown,
): Promise<T> {
  const ownsSession = captureSession();
  // Only the server's session_expired ends the session; its other 401s are ordinary
  // refusals. A 401 that isn't the server's JSON comes from a proxy in front of it.
  const invalidateExpired = (status: number, code: string | undefined) => {
    if (
      status === 401 &&
      (code === 'session_expired' || code === undefined) &&
      ownsSession()
    )
      window.dispatchEvent(new Event('thelxinoe-session-expired'));
  };
  if (desktop) {
    const response = await invoke<{
      status: number;
      body: { error?: { code: string; message: string } };
    }>('backend_request', { path, method, body: body ?? null });
    if (response.status >= 400) {
      invalidateExpired(response.status, response.body.error?.code);
      throw new ApiError(
        response.status,
        response.body.error?.code ?? 'request_failed',
        response.body.error?.message ?? 'Request failed',
      );
    }
    return response.body as T;
  }
  const response = await fetch(`${serverUrl()}/api/v1${path}`, {
    method,
    credentials: 'include',
    headers: {
      'X-Thelxinoe-Client': '1',
      'X-Thelxinoe-API': String(clientApi),
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    // Proxies answer errors such as 413 or 502 with HTML, not the server's JSON.
    const value = await response.json().catch(() => null);
    invalidateExpired(
      response.status,
      value ? (value.error?.code ?? '') : undefined,
    );
    throw new ApiError(
      response.status,
      value?.error?.code ?? 'request_failed',
      value?.error?.message ?? `Request failed (${response.status})`,
    );
  }
  return (await response.json()) as T;
}
// A server update removes the previous build's lazily loaded code. When a page
// cannot load its module, ask which web app the server serves now.
let codeCheck: Promise<void> | undefined;
export function missingWebCode() {
  if (desktop) return;
  return (codeCheck ??= request<{ version?: string }>('/health')
    .then(({ version }) => {
      if (version && version !== clientVersion)
        window.dispatchEvent(
          new CustomEvent('thelxinoe-web-code-missing', { detail: version }),
        );
    })
    .catch(() => {})
    .finally(() => (codeCheck = undefined)));
}
export type ServerEvent = { id: number; kind: string; payload: unknown };
const stallTimeout = 10000;
type EventTicket = {
  ticket: string;
  cursor?: number;
  epoch?: string;
  version?: string;
};
export class Events {
  private socket?: WebSocket;
  private timer?: ReturnType<typeof setTimeout>;
  private closed = false;
  // Each connection attempt; a stalled or replaced one no longer reports.
  private attempt = 0;
  // The attempt waiting for its ticket.
  private issuing?: number;
  private cursor = 0;
  private initialized = false;
  // The ticket `prepare` requested, until a connection uses it.
  private prepared?: Promise<EventTicket>;
  // A prepared ticket that answered after pages had started reading.
  private late?: Promise<EventTicket>;
  // Whether pages may hold data from before the cursor, and must reload once
  // the socket opens.
  private stale = true;
  private epoch?: string;
  private retry = 500;
  // Phones suspend background pages and can leave their connection stalled.
  // Reconnect as soon as the page or the network is back, not after a backoff.
  private resume = () => {
    if (
      this.closed ||
      document.visibilityState !== 'visible' ||
      this.socket?.readyState === WebSocket.OPEN ||
      // An attempt under way keeps its ticket, which may be the prepared one.
      // Its stall timer replaces it if it hangs.
      this.socket?.readyState === WebSocket.CONNECTING ||
      this.issuing === this.attempt
    )
      return;
    this.retry = 500;
    void this.connect();
  };
  constructor(
    private receive: (event: ServerEvent) => void,
    private status: (connected: boolean) => void = () => {},
  ) {
    document.addEventListener('visibilitychange', this.resume);
    window.addEventListener('pageshow', this.resume);
    window.addEventListener('online', this.resume);
  }
  // Captures the event cursor, waiting no longer than a connection may stall.
  // Data read after a timely ticket is covered by the first connection's
  // replay, so that connection asks no page to reload.
  async prepare() {
    const ticket = (this.prepared ??= api<EventTicket>(
      '/auth/event-ticket',
      'POST',
    ));
    let timer: ReturnType<typeof setTimeout> | undefined;
    await Promise.race([
      ticket.catch(() => {}),
      new Promise<void>((resolve) => {
        timer = setTimeout(() => {
          this.late = ticket;
          resolve();
        }, stallTimeout);
      }),
    ]);
    clearTimeout(timer);
  }
  async connect() {
    if (this.closed) return;
    clearTimeout(this.timer);
    const attempt = ++this.attempt;
    const current = () => !this.closed && attempt === this.attempt;
    this.socket?.close();
    this.socket = undefined;
    // An attempt that has not opened by now has stalled; start another.
    this.timer = setTimeout(() => {
      if (current()) this.retryLater();
    }, stallTimeout);
    let ticket: string;
    const prepared = this.prepared;
    this.prepared = undefined;
    this.issuing = attempt;
    try {
      const issued = await (prepared ??
        api<EventTicket>('/auth/event-ticket', 'POST'));
      if (!current()) return;
      this.issuing = undefined;
      ticket = issued.ticket;
      if (
        !this.initialized ||
        issued.epoch !== this.epoch ||
        (issued.cursor ?? 0) < this.cursor
      ) {
        this.cursor = issued.cursor ?? 0;
        this.initialized = true;
        this.stale = !prepared || prepared === this.late;
      }
      this.epoch = issued.epoch;
      if (!desktop && issued.version && issued.version !== clientVersion)
        window.dispatchEvent(
          new CustomEvent('thelxinoe-web-update', { detail: issued.version }),
        );
    } catch {
      if (current()) this.retryLater();
      return;
    }
    const url = new URL(`${serverUrl() || location.origin}/api/v1/events`);
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    url.searchParams.set('since', String(this.cursor));
    url.searchParams.set('ticket', ticket);
    const socket = new WebSocket(url);
    this.socket = socket;
    socket.onopen = () => {
      if (!current()) return;
      clearTimeout(this.timer);
      this.retry = 500;
      this.status(true);
      if (this.stale)
        this.receive({
          id: this.cursor,
          kind: 'server.reconnected',
          payload: {},
        });
      this.stale = true;
    };
    socket.onmessage = (message) => {
      if (!current()) return;
      const event = JSON.parse(message.data) as ServerEvent;
      if (event.id > this.cursor) {
        this.cursor = event.id;
        this.receive(event);
      }
    };
    socket.onclose = () => {
      if (current()) this.retryLater();
    };
  }
  private retryLater() {
    this.attempt++;
    this.socket?.close();
    this.socket = undefined;
    this.status(false);
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.connect(), this.retry);
    this.retry = Math.min(this.retry * 2, 15000);
  }
  close() {
    this.closed = true;
    clearTimeout(this.timer);
    this.socket?.close();
    document.removeEventListener('visibilitychange', this.resume);
    window.removeEventListener('pageshow', this.resume);
    window.removeEventListener('online', this.resume);
  }
}
