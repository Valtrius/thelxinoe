import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => vi.fn());
vi.mock('../api', () => ({ api }));

import { invalidateSession } from '../session';
import { forgetSeerrStatus, seerrStatus, seerrTitle } from './status';

describe('Seerr status', () => {
  beforeEach(() => {
    forgetSeerrStatus();
    api.mockReset();
  });

  it('asks once while Seerr is ready', async () => {
    api.mockResolvedValue({ configured: true, ready: true });
    await seerrStatus();
    await expect(seerrStatus()).resolves.toEqual({
      configured: true,
      ready: true,
    });
    expect(api).toHaveBeenCalledTimes(1);
    forgetSeerrStatus();
    await seerrStatus();
    invalidateSession();
    await seerrStatus();
    expect(api).toHaveBeenCalledTimes(3);
  });

  it('asks again until Seerr is ready', async () => {
    api.mockResolvedValueOnce({ configured: false });
    await seerrStatus();
    api.mockResolvedValueOnce({ configured: true, ready: false });
    await seerrStatus();
    api.mockRejectedValueOnce(new Error('Seerr is unreachable'));
    await expect(seerrStatus()).rejects.toThrow('Seerr is unreachable');
    api.mockResolvedValue({ configured: true, ready: true });
    await seerrStatus();
    await seerrStatus();
    expect(api).toHaveBeenCalledTimes(4);
  });

  it('reads each request title once a session unless asked again', async () => {
    api.mockRejectedValueOnce(new Error('Seerr is unreachable'));
    await expect(seerrTitle('movie', 1)).rejects.toThrow();
    api.mockResolvedValue({ id: 1, title: 'Arrival' });
    await seerrTitle('movie', 1);
    await expect(seerrTitle('movie', 1)).resolves.toEqual({
      id: 1,
      title: 'Arrival',
    });
    expect(api).toHaveBeenCalledTimes(2);
    expect(api).toHaveBeenLastCalledWith('/seerr/movie/1');
    await seerrTitle('tv', 1);
    await seerrTitle('movie', 1, true);
    expect(api).toHaveBeenCalledTimes(4);
    invalidateSession();
    await seerrTitle('movie', 1);
    expect(api).toHaveBeenCalledTimes(5);
  });
});
