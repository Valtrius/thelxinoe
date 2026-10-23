import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { RefreshTask } from './refresh-task';

let page: EventTarget & { hidden: boolean };
beforeEach(() => {
  vi.useFakeTimers();
  page = Object.assign(new EventTarget(), { hidden: false });
  vi.stubGlobal('document', page);
  vi.stubGlobal('window', new EventTarget());
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
function visible(value: boolean) {
  page.hidden = !value;
  page.dispatchEvent(new Event('visibilitychange'));
}
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => (resolve = done));
  return { promise, resolve };
}

describe('visible-page refreshes', () => {
  it('waits for slow reads instead of accumulating periodic catch-ups', async () => {
    const pending = deferred();
    const load = vi.fn(() => pending.promise);
    const task = new RefreshTask(load, 1000);
    task.start();
    expect(load).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10_000);
    window.dispatchEvent(new Event('focus'));
    expect(load).toHaveBeenCalledTimes(1);
    pending.resolve();
    await vi.advanceTimersByTimeAsync(999);
    expect(load).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(load).toHaveBeenCalledTimes(2);
    task.stop();
  });

  it('coalesces changes during a read and waits for the catch-up', async () => {
    const first = deferred(),
      second = deferred();
    const load = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValue(second.promise);
    const task = new RefreshTask(load, 1000);
    let finished = false;
    const request = task.refresh().then(() => (finished = true));
    void task.refresh();
    void task.refresh();
    expect(load).toHaveBeenCalledTimes(1);
    first.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(load).toHaveBeenCalledTimes(2);
    expect(finished).toBe(false);
    second.resolve();
    await request;
    expect(finished).toBe(true);
    task.stop();
  });

  it('defers hidden changes and coalesces visibility, focus and reconnect', async () => {
    const load = vi.fn(async () => {});
    const task = new RefreshTask(load, 1000);
    task.start();
    await vi.advanceTimersByTimeAsync(0);
    visible(false);
    await task.refresh();
    await task.refresh();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(load).toHaveBeenCalledTimes(1);
    visible(true);
    window.dispatchEvent(new Event('focus'));
    window.dispatchEvent(new Event('online'));
    await vi.advanceTimersByTimeAsync(0);
    expect(load).toHaveBeenCalledTimes(2);
    window.dispatchEvent(new Event('focus'));
    await vi.advanceTimersByTimeAsync(1000);
    expect(load).toHaveBeenCalledTimes(3);
    task.stop();
  });

  it('resumes its timer after a short hidden period without an immediate read', async () => {
    const load = vi.fn(async () => {});
    const task = new RefreshTask(load, 1000);
    task.start();
    await vi.advanceTimersByTimeAsync(10);
    visible(false);
    await vi.advanceTimersByTimeAsync(50);
    visible(true);
    expect(load).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(load).toHaveBeenCalledTimes(2);
    task.stop();
  });

  it('refreshes overdue data on return even when hidden timers never fired', async () => {
    const load = vi.fn(async () => {});
    const task = new RefreshTask(load, 1000);
    task.start();
    await vi.advanceTimersByTimeAsync(0);
    visible(false);
    vi.setSystemTime(Date.now() + 60_000);
    visible(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(load).toHaveBeenCalledTimes(2);
    task.stop();
  });

  it('does not start a queued read after disposal or while hidden', async () => {
    for (const stop of [false, true]) {
      visible(true);
      const pending = deferred();
      const load = vi.fn(() => pending.promise);
      const task = new RefreshTask(load, 1000);
      task.start();
      void task.refresh();
      if (stop) task.stop();
      else visible(false);
      pending.resolve();
      await vi.advanceTimersByTimeAsync(5000);
      expect(load).toHaveBeenCalledTimes(1);
      visible(true);
      window.dispatchEvent(new Event('online'));
      await vi.advanceTimersByTimeAsync(0);
      expect(load).toHaveBeenCalledTimes(stop ? 1 : 2);
      task.stop();
    }
  });

  it('recovers from failures and supports activity-dependent polling', async () => {
    let active = false;
    const load = vi
      .fn()
      .mockRejectedValueOnce(Error('offline'))
      .mockResolvedValue(undefined);
    const task = new RefreshTask(load, 1000, () => active);
    task.start();
    await vi.advanceTimersByTimeAsync(5000);
    expect(load).toHaveBeenCalledTimes(1);
    await task.refresh();
    expect(load).toHaveBeenCalledTimes(2);
    active = true;
    await vi.advanceTimersByTimeAsync(1000);
    expect(load).toHaveBeenCalledTimes(3);
    task.stop();
  });
});
