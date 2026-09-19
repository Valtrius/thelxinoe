import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, Events } from './api';
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
describe('server transport', () => {
  it('normalizes server errors and sends the CSRF header without credential URLs', async () => {
    const fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 403,
      json: async () => ({
        error: { code: 'forbidden', message: 'Access denied' },
      }),
    });
    vi.stubGlobal('fetch', fetch);
    await expect(
      api('/users', 'POST', { username: 'test' }),
    ).rejects.toMatchObject({ status: 403, code: 'forbidden' });
    expect(fetch.mock.calls[0][0]).toBe('/api/v1/users');
    expect(fetch.mock.calls[0][1].headers['X-Thelxinoe-Client']).toBe('1');
  });
  it('reconnects with an event cursor and a fresh ticket, then stops cleanly', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('location', { origin: 'https://media.test' });
    let sequence = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async () => ({
        ok: true,
        json: async () => ({ ticket: `ticket${++sequence}` }),
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
    Socket.all[0].onopen?.();
    Socket.all[0].onmessage?.({
      data: JSON.stringify({ id: 7, kind: 'catalog.changed', payload: {} }),
    });
    Socket.all[0].onmessage?.({
      data: JSON.stringify({ id: 6, kind: 'catalog.changed', payload: {} }),
    });
    expect(receive).toHaveBeenCalledTimes(1);
    Socket.all[0].close();
    await vi.advanceTimersByTimeAsync(500);
    expect(Socket.all).toHaveLength(2);
    expect(Socket.all[1].url).toContain('since=7');
    expect(Socket.all[1].url).toContain('ticket=ticket2');
    events.close();
    await vi.advanceTimersByTimeAsync(30000);
    expect(Socket.all).toHaveLength(2);
  });
});
