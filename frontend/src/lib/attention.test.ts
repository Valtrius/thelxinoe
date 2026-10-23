import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { api } from './api';
import {
  attention,
  connectAttention,
  refreshAttention,
  type AttentionItem,
} from './attention';

vi.mock('./api', () => ({ api: vi.fn() }));
const read = vi.mocked(api);
let page: EventTarget & { hidden: boolean };
const stops: (() => void)[] = [];
const item = (id: string): AttentionItem => ({
  id,
  target: 'services',
  severity: 'error',
  resource: 'radarr',
  message: id,
  revision: id,
  dismissible: false,
  media_ids: [],
});
beforeEach(() => {
  vi.useFakeTimers();
  page = Object.assign(new EventTarget(), { hidden: false });
  vi.stubGlobal('document', page);
  vi.stubGlobal('window', new EventTarget());
  read.mockReset();
});
afterEach(() => {
  stops.splice(0).forEach((stop) => stop());
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('retains hidden attention changes and fetches the latest state once on return', async () => {
  read.mockResolvedValue({ items: [item('before')] });
  stops.push(connectAttention('one'));
  await vi.advanceTimersByTimeAsync(0);
  expect(get(attention)[0].id).toBe('before');
  page.hidden = true;
  document.dispatchEvent(new Event('visibilitychange'));
  read.mockResolvedValue({ items: [item('after')] });
  for (let i = 0; i < 10; i++) await refreshAttention();
  await vi.advanceTimersByTimeAsync(60_000);
  expect(read).toHaveBeenCalledTimes(1);
  expect(get(attention)[0].id).toBe('before');
  page.hidden = false;
  document.dispatchEvent(new Event('visibilitychange'));
  window.dispatchEvent(new Event('focus'));
  await vi.advanceTimersByTimeAsync(0);
  expect(read).toHaveBeenCalledTimes(2);
  expect(get(attention)[0].id).toBe('after');
});

it('does not publish or repeat a previous session read after reconnecting attention', async () => {
  let finish!: (value: unknown) => void;
  read.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const previous = connectAttention('one');
  void refreshAttention();
  read.mockResolvedValue({ items: [item('current')] });
  stops.push(connectAttention('two'));
  previous();
  await vi.advanceTimersByTimeAsync(0);
  finish({ items: [item('obsolete')] });
  await vi.advanceTimersByTimeAsync(0);
  expect(read).toHaveBeenCalledTimes(2);
  expect(get(attention)[0].id).toBe('current');
  await refreshAttention();
  expect(read).toHaveBeenCalledTimes(3);
});
