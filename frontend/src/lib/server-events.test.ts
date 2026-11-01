import { expect, it, vi } from 'vitest';
import { onServerEvent, publish, revisions } from './server-events.svelte';

const event = (kind: string) => ({ id: 1, kind, payload: {} });

it('moves each revision a reconnect covers once', () => {
  const before = { ...revisions };
  publish(event('server.reconnected'));
  expect(revisions).toEqual({
    media: before.media + 1,
    catalog: before.catalog + 1,
    account: before.account + 1,
    preferences: before.preferences,
    settings: before.settings + 1,
  });
});

it('lets playback progress reach listeners without reloading pages', () => {
  const before = { ...revisions };
  const listener = vi.fn();
  const stop = onServerEvent(listener);
  publish(event('playback.changed'), true);
  expect(revisions).toEqual(before);
  expect(listener).toHaveBeenCalledWith(event('playback.changed'));
  stop();
  publish(event('playback.changed'));
  expect(listener).toHaveBeenCalledTimes(1);
  expect(revisions.media).toBe(before.media + 1);
});
