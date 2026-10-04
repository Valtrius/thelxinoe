import { invoke, isTauri } from '@tauri-apps/api/core';
import { version as clientVersion } from '../../package.json';
import { captureSession, invalidateSession } from './session';
export const desktop = typeof window !== 'undefined' && isTauri();
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
}
export function serverUrl(): string {
  return desktop ? desktopServer : '';
}
export async function api<T>(
  path: string,
  method = 'GET',
  body?: unknown,
): Promise<T> {
  const ownsSession = captureSession();
  const invalidateExpired = (status: number) => {
    if (
      status === 401 &&
      ownsSession() &&
      path !== '/auth/login' &&
      !path.startsWith('/auth/totp') &&
      !path.startsWith('/auth/passkey') &&
      !path.startsWith('/auth/recovery') &&
      !path.startsWith('/auth/desktop/') &&
      path !== '/auth/verify' &&
      path !== '/setup'
    )
      window.dispatchEvent(new Event('thelxinoe-session-expired'));
  };
  if (desktop) {
    const response = await invoke<{
      status: number;
      body: { error?: { code: string; message: string } };
    }>('backend_request', { path, method, body: body ?? null });
    if (response.status >= 400) {
      invalidateExpired(response.status);
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
      'X-Thelxinoe-API': '1',
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const value = await response.json();
  if (!response.ok) {
    invalidateExpired(response.status);
    throw new ApiError(
      response.status,
      value.error?.code ?? 'request_failed',
      value.error?.message ?? 'Request failed',
    );
  }
  return value as T;
}
export type ServerEvent = { id: number; kind: string; payload: unknown };
export class Events {
  private socket?: WebSocket;
  private timer?: ReturnType<typeof setTimeout>;
  private closed = false;
  private cursor = 0;
  private initialized = false;
  private epoch?: string;
  private retry = 500;
  constructor(
    private receive: (event: ServerEvent) => void,
    private status: (connected: boolean) => void = () => {},
  ) {}
  async connect() {
    if (this.closed) return;
    let ticket: string;
    try {
      const issued = await api<{
        ticket: string;
        cursor?: number;
        epoch?: string;
        version?: string;
      }>('/auth/event-ticket', 'POST');
      ticket = issued.ticket;
      if (
        !this.initialized ||
        issued.epoch !== this.epoch ||
        (issued.cursor ?? 0) < this.cursor
      ) {
        this.cursor = issued.cursor ?? 0;
        this.initialized = true;
      }
      this.epoch = issued.epoch;
      if (!desktop && issued.version && issued.version !== clientVersion)
        window.dispatchEvent(
          new CustomEvent('thelxinoe-web-update', { detail: issued.version }),
        );
    } catch {
      if (!this.closed)
        this.timer = setTimeout(() => void this.connect(), 15000);
      return;
    }
    if (this.closed) return;
    const url = new URL(`${serverUrl() || location.origin}/api/v1/events`);
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    url.searchParams.set('since', String(this.cursor));
    url.searchParams.set('ticket', ticket);
    this.socket = new WebSocket(url);
    this.socket.onopen = () => {
      this.retry = 500;
      this.status(true);
      this.receive({
        id: this.cursor,
        kind: 'server.reconnected',
        payload: {},
      });
    };
    this.socket.onmessage = (message) => {
      const event = JSON.parse(message.data) as ServerEvent;
      if (event.id > this.cursor) {
        this.cursor = event.id;
        this.receive(event);
      }
    };
    this.socket.onclose = () => {
      this.status(false);
      if (!this.closed) {
        this.timer = setTimeout(() => this.connect(), this.retry);
        this.retry = Math.min(this.retry * 2, 15000);
      }
    };
  }
  close() {
    this.closed = true;
    clearTimeout(this.timer);
    this.socket?.close();
  }
}
