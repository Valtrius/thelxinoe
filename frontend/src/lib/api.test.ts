import { afterEach, describe, expect, it, vi } from 'vitest';
import { version as clientVersion } from '../../package.json';
import { ApiError, Events, api, clientApi, missingWebCode } from './api';
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
describe('server transport', () => {
  it('reconnects with an event cursor and a fresh ticket, then stops cleanly', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('location', { origin: 'https://media.test' });
    let sequence = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async () => ({
        ok: true,
        json: async () => ({
          ticket: `ticket${++sequence}`,
          cursor: sequence === 1 ? 100 : 2000,
        }),
      })),
    );
    class Socket {
      static all: Socket[] = [];
      onopen?: () => void;
      onclose?: () => void;
      onmessage?: (message: { data: string }) => void;
      url: string;
      constructor(url: string | URL) {
        this.url = String(url);
        Socket.all.push(this);
      }
      close() {
        this.onclose?.();
      }
    }
    vi.stubGlobal('WebSocket', Socket);
    const receive = vi.fn();
    const events = new Events(receive);
    await events.connect();
    expect(Socket.all[0].url).toContain('since=100');
    Socket.all[0].onopen?.();
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
