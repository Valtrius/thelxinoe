import { invoke, isTauri } from '@tauri-apps/api/core';
export const desktop = typeof window !== 'undefined' && isTauri();
let desktopServer = '';
export async function initializeTransport() {
  if (desktop) desktopServer = await invoke<string>('server_url');
}
export async function changeServer(value: string) {
  if (desktop) desktopServer = await invoke<string>('change_server', { value });
}
export type User = {
  id: string;
  username: string;
  role: 'admin' | 'user';
  timezone: string;
};
export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
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
  if (desktop) {
    const response = await invoke<{
      status: number;
      body: { error?: { code: string; message: string } };
    }>('backend_request', { path, method, body: body ?? null });
    if (response.status >= 400)
      throw new ApiError(
        response.status,
        response.body.error?.code ?? 'request_failed',
        response.body.error?.message ?? 'Request failed',
      );
    return response.body as T;
  }
  const response = await fetch(`${serverUrl()}/api/v1${path}`, {
    method,
    credentials: 'include',
    headers: {
      'X-Thelxinoe-Client': '1',
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const value = await response.json();
  if (!response.ok)
    throw new ApiError(
      response.status,
      value.error?.code ?? 'request_failed',
      value.error?.message ?? 'Request failed',
    );
  return value as T;
}
export type ServerEvent = { id: number; kind: string; payload: unknown };
export class Events {
  private socket?: WebSocket;
  private timer?: ReturnType<typeof setTimeout>;
  private closed = false;
  private cursor = 0;
  private initialized = false;
  private retry = 500;
  constructor(
    private receive: (event: ServerEvent) => void,
    private status: (connected: boolean) => void = () => {},
  ) {}
  async connect() {
    if (this.closed) return;
    let ticket: string;
    try {
      const issued = await api<{ ticket: string; cursor?: number }>(
        '/auth/event-ticket',
        'POST',
      );
      ticket = issued.ticket;
      if (!this.initialized) {
        this.cursor = issued.cursor ?? 0;
        this.initialized = true;
      }
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
