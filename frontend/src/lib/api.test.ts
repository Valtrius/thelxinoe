import { afterEach, describe, expect, it, vi } from 'vitest';
import { version as clientVersion } from '../../package.json';
import { ApiError, Events, api, clientApi, missingWebCode } from './api';
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
class Socket {
  static CONNECTING = 0;
  static OPEN = 1;
  static all: Socket[] = [];
  readyState = 0;
  onopen?: () => void;
  onclose?: () => void;
  onmessage?: (message: { data: string }) => void;
  url: string;
  constructor(url: string | URL) {
    this.url = String(url);
    Socket.all.push(this);
  }
  open() {
    this.readyState = Socket.OPEN;
    this.onopen?.();
  }
  close() {
    this.readyState = 3;
    this.onclose?.();
  }
}
// A browser page whose sockets and ticket requests are under the test's control.
function page(fetch = vi.fn()) {
  vi.useFakeTimers();
  Socket.all = [];
  const document = Object.assign(new EventTarget(), {
    visibilityState: 'visible',
  });
  vi.stubGlobal('document', document);
  vi.stubGlobal('window', new EventTarget());
  vi.stubGlobal('location', { origin: 'https://media.test' });
  vi.stubGlobal('WebSocket', Socket);
  let sequence = 0;
  vi.stubGlobal(
    'fetch',
    fetch.mockImplementation(async () => ({
      ok: true,
      json: async () => ({
        ticket: `ticket${++sequence}`,
        cursor: sequence === 1 ? 100 : 2000,
      }),
    })),
  );
  return document;
}
describe('server transport', () => {
  it('reconnects with an event cursor and a fresh ticket, then stops cleanly', async () => {
    page();
    const receive = vi.fn();
    const events = new Events(receive);
    await events.connect();
    expect(Socket.all[0].url).toContain('since=100');
    Socket.all[0].open();
    receive.mockClear();
    Socket.all[0].onmessage?.({
      data: JSON.stringify({ id: 107, kind: 'catalog.changed', payload: {} }),
    });
    Socket.all[0].onmessage?.({
      data: JSON.stringify({ id: 106, kind: 'catalog.changed', payload: {} }),
    });
    expect(receive).toHaveBeenCalledTimes(1);
    Socket.all[0].close();
    await vi.advanceTimersByTimeAsync(500);
    expect(Socket.all).toHaveLength(2);
    expect(Socket.all[1].url).toContain('since=107');
    expect(Socket.all[1].url).toContain('ticket=ticket2');
    events.close();
    await vi.advanceTimersByTimeAsync(30000);
    expect(Socket.all).toHaveLength(2);
  });
  it('asks pages to reload when their data may predate the cursor', async () => {
    page();
    const receive = vi.fn();
    const events = new Events(receive);
    await events.connect();
    Socket.all[0].open();
    expect(receive).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'server.reconnected' }),
    );
    events.close();
  });
  it('replays instead of reloading data read after the prepared cursor', async () => {
    page();
    const receive = vi.fn();
    const events = new Events(receive);
    await events.prepare();
    void events.connect();
    await vi.advanceTimersByTimeAsync(0);
    expect(Socket.all[0].url).toContain('since=100');
    Socket.all[0].open();
    expect(receive).not.toHaveBeenCalled();
    // A later connection may have missed changes the server no longer replays.
    Socket.all[0].close();
    await vi.advanceTimersByTimeAsync(500);
    Socket.all[1].open();
    expect(receive).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'server.reconnected' }),
    );
    events.close();
  });
  it('keeps the prepared ticket when the page is shown while connecting', async () => {
    const fetch = vi.fn();
    page(fetch);
    let answer!: () => void;
    const answered = new Promise<void>((resolve) => (answer = resolve));
    const respond = fetch.getMockImplementation()!;
    fetch.mockImplementationOnce(async () => {
      await answered;
      return respond();
    });
    const receive = vi.fn();
    const events = new Events(receive);
    const prepared = events.prepare();
    void events.connect();
    // The first page load's pageshow can land while startup still waits.
    window.dispatchEvent(new Event('pageshow'));
    answer();
    await prepared;
    await vi.advanceTimersByTimeAsync(0);
    window.dispatchEvent(new Event('pageshow'));
    await vi.advanceTimersByTimeAsync(0);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(Socket.all).toHaveLength(1);
    Socket.all[0].open();
    expect(receive).not.toHaveBeenCalled();
    events.close();
  });
  it('reloads pages when the prepared ticket failed', async () => {
    const fetch = vi.fn();
    page(fetch);
    fetch.mockRejectedValueOnce(new TypeError('Network connection was lost'));
    const receive = vi.fn();
    const events = new Events(receive);
    await events.prepare();
    await events.connect();
    await vi.advanceTimersByTimeAsync(500);
    Socket.all[0].open();
    expect(receive).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'server.reconnected' }),
    );
    events.close();
  });
  it('stops waiting for a prepared ticket that stalls, then reloads pages', async () => {
    const fetch = vi.fn();
    page(fetch);
    let answer!: () => void;
    const answered = new Promise<void>((resolve) => (answer = resolve));
    const respond = fetch.getMockImplementation()!;
    fetch.mockImplementationOnce(async () => {
      await answered;
      return respond();
    });
    const receive = vi.fn();
    const events = new Events(receive);
    let ready = false;
    void events.prepare().then(() => (ready = true));
    void events.connect();
    await vi.advanceTimersByTimeAsync(9999);
    expect(ready).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(ready).toBe(true);
    answer();
    await vi.advanceTimersByTimeAsync(500);
    Socket.all.at(-1)!.open();
    expect(receive).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'server.reconnected' }),
    );
    events.close();
  });
  it('replaces a connection that stalls before opening', async () => {
    page();
    const status = vi.fn();
    const events = new Events(() => {}, status);
    await events.connect();
    await vi.advanceTimersByTimeAsync(9999);
    expect(Socket.all).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1 + 500);
    expect(status).toHaveBeenLastCalledWith(false);
    expect(Socket.all).toHaveLength(2);
    expect(Socket.all[0].readyState).toBe(3);
    Socket.all[1].open();
    expect(status).toHaveBeenLastCalledWith(true);
    // The stalled socket closing late cannot mark the open one disconnected.
    Socket.all[0].onclose?.();
    expect(status).toHaveBeenLastCalledWith(true);
    events.close();
  });
  it('retries a ticket request that never answers', async () => {
    const fetch = vi.fn();
    page(fetch);
    fetch.mockImplementationOnce(() => new Promise(() => {}));
    const events = new Events(() => {});
    void events.connect();
    await vi.advanceTimersByTimeAsync(10500);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(Socket.all).toHaveLength(1);
    events.close();
  });
  it('reconnects as soon as a suspended page is shown again', async () => {
    const fetch = vi.fn();
    const document = page(fetch);
    const status = vi.fn();
    const events = new Events(() => {}, status);
    await events.connect();
    Socket.all[0].open();
    // A phone drops the connection in the background, and its retries fail
    // until the next one waits the longest backoff.
    document.visibilityState = 'hidden';
    const answer = fetch.getMockImplementation()!;
    fetch.mockRejectedValue(new TypeError('Network connection was lost'));
    Socket.all[0].close();
    await vi.advanceTimersByTimeAsync(500 + 1000 + 2000 + 4000 + 8000);
    expect(status).toHaveBeenLastCalledWith(false);
    fetch.mockImplementation(answer);
    document.visibilityState = 'visible';
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(Socket.all).toHaveLength(2);
    Socket.all[1].open();
    expect(status).toHaveBeenLastCalledWith(true);
    // An open connection is left alone.
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(Socket.all).toHaveLength(2);
    events.close();
  });
  it('stops listening to the page when closed', async () => {
    const document = page();
    const events = new Events(() => {});
    await events.connect();
    events.close();
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(30000);
    expect(Socket.all).toHaveLength(1);
  });
  it('reports a proxy error page by status and still expires the session', async () => {
    const target = new EventTarget();
    vi.stubGlobal('window', target);
    let expired = 0;
    target.addEventListener('thelxinoe-session-expired', () => expired++);
    const html = (status: number) => ({
      ok: false,
      status,
      json: async () => JSON.parse('<html>Bad gateway</html>'),
    });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(html(502)));
    const failed = await api('/library').catch((error: unknown) => error);
    expect(failed).toBeInstanceOf(ApiError);
    expect(String(failed)).toBe('Request failed (502)');
    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(html(401)));
    await api('/library').catch(() => undefined);
    expect(expired).toBe(1);
  });
  it('signs out only when the server says the session ended', async () => {
    const target = new EventTarget();
    vi.stubGlobal('window', target);
    let expired = 0;
    target.addEventListener('thelxinoe-session-expired', () => expired++);
    const refusal = (code: string) => ({
      ok: false,
      status: 401,
      json: async () => ({ error: { code, message: 'Refused' } }),
    });
    // An expired Quick Connect approval or a removed passkey isn't the session ending.
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValueOnce(refusal('unauthenticated')),
    );
    const failed = await api('/auth/quick-connect/approve', 'POST', {}).catch(
      (error: unknown) => error,
    );
    expect(String(failed)).toBe('Refused');
    expect(expired).toBe(0);
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValueOnce(refusal('session_expired')),
    );
    await api('/library').catch(() => undefined);
    expect(expired).toBe(1);
  });
  it('shows server explanations without an error-type prefix', () => {
    const error = new ApiError(409, 'conflict', 'Radarr rejected the API key');
    expect(String(error)).toBe('Radarr rejected the API key');
    expect(error.code).toBe('conflict');
  });
  it('sends its API version and reports newer web code after a missing chunk', async () => {
    const target = new EventTarget();
    vi.stubGlobal('window', target);
    const updates: string[] = [];
    target.addEventListener('thelxinoe-web-code-missing', (event) =>
      updates.push((event as CustomEvent<string>).detail),
    );
    let served = clientVersion;
    const fetch = vi.fn().mockImplementation(async () => ({
      ok: true,
      json: async () => ({ version: served }),
    }));
    vi.stubGlobal('fetch', fetch);
    await missingWebCode();
    expect(updates).toEqual([]);
    expect(fetch.mock.calls[0][1].headers['X-Thelxinoe-API']).toBe(
      String(clientApi),
    );
    served = '99.0.0';
    // Several pages failing together ask the server once.
    await Promise.all([missingWebCode(), missingWebCode()]);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(updates).toEqual(['99.0.0']);
  });
});
