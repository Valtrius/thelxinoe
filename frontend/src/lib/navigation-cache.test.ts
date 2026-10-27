import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import {
  NavigationCache,
  navigationCache,
  invalidatePageEvent,
  invalidatePageWrite,
} from './navigation-cache';
import { invalidateSession } from './session';

beforeEach(() => {
  vi.useFakeTimers();
  navigationCache.clear();
});
afterEach(() => {
  vi.useRealTimers();
  navigationCache.clear();
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => (resolve = done));
  return { promise, resolve };
}

it('reuses recent reads, isolates readers and distinguishes filters and sources', async () => {
  const cache = new NavigationCache();
  const load = vi.fn(async () => ({ items: ['first'] }));
  const accept = vi.fn((value: { items: string[] }) => {
    value.items.push('edited');
  });
  await cache.read('catalog', 'q=one', load, accept);
  const restored = vi.fn();
  const reading = cache.read('catalog', 'q=one', load, restored);
  expect(restored).toHaveBeenCalledWith({ items: ['first'] });
  await reading;
  expect(load).toHaveBeenCalledTimes(1);
  await cache.read('catalog', 'q=two', load, restored);
  await cache.read('discover', 'q=one', load, restored);
  expect(load).toHaveBeenCalledTimes(3);
});

it('preserves invalidation triggered while a view accepts cached data', async () => {
  const cache = new NavigationCache();
  const load = vi.fn(async () => ['account']);
  await cache.read('youtube', 'account', load, vi.fn());
  await cache.read('youtube', 'account', load, () =>
    cache.invalidate('youtube'),
  );
  await cache.read('youtube', 'account', load, vi.fn());
  expect(load).toHaveBeenCalledTimes(2);
});

it('shows stale data before revalidation and expires old snapshots', async () => {
  const cache = new NavigationCache();
  const accept = vi.fn();
  await cache.read('catalog', 'movies', async () => ['before'], accept);
  vi.advanceTimersByTime(30_001);
  const pending = deferred<string[]>();
  const reading = cache.read(
    'catalog',
    'movies',
    () => pending.promise,
    accept,
  );
  expect(accept).toHaveBeenLastCalledWith(['before']);
  pending.resolve(['after']);
  await reading;
  expect(accept).toHaveBeenLastCalledWith(['after']);
  vi.advanceTimersByTime(300_001);
  accept.mockClear();
  const next = cache.read('catalog', 'movies', async () => ['expired'], accept);
  expect(accept).not.toHaveBeenCalled();
  await next;
  expect(accept).toHaveBeenCalledWith(['expired']);
});

it('shares pending reads but does not retain a response invalidated during its request', async () => {
  const cache = new NavigationCache();
  const pending = deferred<string[]>();
  const load = vi.fn(() => pending.promise);
  const first = vi.fn(),
    second = vi.fn();
  const one = cache.read('catalog', 'movies', load, first);
  const two = cache.read('catalog', 'movies', load, second);
  expect(load).toHaveBeenCalledTimes(1);
  cache.invalidate('catalog');
  await cache.read('catalog', 'movies', async () => ['newer'], vi.fn());
  pending.resolve(['old']);
  await Promise.all([one, two]);
  const restored = vi.fn();
  await cache.read('catalog', 'movies', load, restored);
  expect(restored).toHaveBeenCalledWith(['newer']);
  expect(load).toHaveBeenCalledTimes(1);
  expect(first.mock.calls[0][0]).not.toBe(second.mock.calls[0][0]);
});

it('clears snapshots and ignores late responses when the session changes', async () => {
  const pending = deferred<string[]>();
  const accept = vi.fn();
  await navigationCache.read(
    'catalog',
    'warm',
    async () => ['private'],
    vi.fn(),
  );
  const request = navigationCache.read(
    'youtube',
    'pending',
    () => pending.promise,
    accept,
  );
  invalidateSession();
  pending.resolve(['previous session']);
  await request;
  expect(accept).not.toHaveBeenCalled();
  const load = vi.fn(async () => ['current']);
  await navigationCache.read('catalog', 'warm', load, accept);
  expect(load).toHaveBeenCalledTimes(1);
  expect(accept).toHaveBeenCalledWith(['current']);
});

it('bounds retained entries and bytes and refreshes explicitly on demand', async () => {
  const cache = new NavigationCache(200, 2);
  const load = vi.fn(async () => ['small']);
  for (const key of ['a', 'b', 'a', 'c', 'b'])
    await cache.read('catalog', key, load, vi.fn());
  expect(load).toHaveBeenCalledTimes(4);
  await cache.read('catalog', 'b', load, vi.fn(), 0);
  expect(load).toHaveBeenCalledTimes(5);
  const large = vi.fn(async () => 'x'.repeat(200));
  await cache.read('catalog', 'large', large, vi.fn());
  await cache.read('catalog', 'large', large, vi.fn());
  expect(large).toHaveBeenCalledTimes(2);
});

it('drops failed revalidations so returning to a failed source retries it', async () => {
  const cache = new NavigationCache();
  await cache.read('catalog', 'movies', async () => ['old'], vi.fn());
  await expect(
    cache.read(
      'catalog',
      'movies',
      async () => {
        throw Error('offline');
      },
      vi.fn(),
      0,
    ),
  ).rejects.toThrow('offline');
  const load = vi.fn(async () => ['retry']);
  await cache.read('catalog', 'movies', load, vi.fn());
  expect(load).toHaveBeenCalledTimes(1);
});

it('invalidates changed resources without treating YouTube browse or playback keepalives as writes', async () => {
  const catalog = vi.fn(async () => ['movies']);
  const youtube = vi.fn(async () => ['videos']);
  const read = async () => {
    await navigationCache.read('catalog', 'movies', catalog, vi.fn());
    await navigationCache.read('youtube', 'videos', youtube, vi.fn());
  };
  await read();
  invalidatePageWrite('/online/youtube/browse');
  invalidatePageWrite('/playback/id/keepalive');
  await read();
  expect(catalog).toHaveBeenCalledTimes(1);
  expect(youtube).toHaveBeenCalledTimes(1);
  invalidatePageEvent('youtube.changed');
  await read();
  expect(catalog).toHaveBeenCalledTimes(1);
  expect(youtube).toHaveBeenCalledTimes(2);
  invalidatePageWrite('/catalog/id/state');
  await read();
  expect(catalog).toHaveBeenCalledTimes(2);
  invalidatePageEvent('server.reconnected');
  await read();
  expect(catalog).toHaveBeenCalledTimes(3);
  expect(youtube).toHaveBeenCalledTimes(3);
});

it('shows snapshots at once after progress reports but drops them when playback changes state', async () => {
  const load = vi.fn(async () => ['continue watching']);
  const pending = deferred<string[]>();
  await navigationCache.read('catalog', '/me/home', load, vi.fn());
  invalidatePageWrite('/playback/id/progress');
  const accept = vi.fn();
  const reading = navigationCache.read(
    'catalog',
    '/me/home',
    () => pending.promise,
    accept,
  );
  expect(accept).toHaveBeenCalledWith(['continue watching']);
  pending.resolve(['moved']);
  await reading;
  expect(accept).toHaveBeenLastCalledWith(['moved']);
  invalidatePageEvent('playback.changed', true);
  accept.mockClear();
  await navigationCache.read('catalog', '/me/home', load, accept);
  expect(accept).toHaveBeenCalledTimes(2);
  expect(load).toHaveBeenCalledTimes(2);
  invalidatePageEvent('playback.changed');
  accept.mockClear();
  await navigationCache.read('catalog', '/me/home', load, accept);
  expect(accept).toHaveBeenCalledTimes(1);
  expect(load).toHaveBeenCalledTimes(3);
});
