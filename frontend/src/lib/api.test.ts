import { afterEach, describe, expect, it, vi } from 'vitest';
import { Events } from './api';
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
});
