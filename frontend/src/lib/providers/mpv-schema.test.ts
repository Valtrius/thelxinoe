import { describe, expect, it, vi } from 'vitest';
import { loadMpvSchema } from './mpv-schema';

describe('MPV option cache', () => {
  it('shares option discovery for the same executable and supports an explicit refresh', async () => {
    const load = vi.fn(async () => ({
      executable: 'example',
      version: '1',
      options: [],
    }));
    await Promise.all([
      loadMpvSchema('same-executable', false, load),
      loadMpvSchema('same-executable', false, load),
    ]);
    expect(load).toHaveBeenCalledTimes(1);
    await loadMpvSchema('same-executable', true, load);
    expect(load).toHaveBeenCalledTimes(2);
    await loadMpvSchema('another-executable', false, load);
    expect(load).toHaveBeenCalledTimes(3);
  });

  it('allows retrying failed discovery', async () => {
    const load = vi
      .fn()
      .mockRejectedValueOnce(new Error('Unavailable'))
      .mockResolvedValue({ executable: 'example', version: '1', options: [] });
    await expect(
      loadMpvSchema('retry-executable', false, load),
    ).rejects.toThrow('Unavailable');
    await loadMpvSchema('retry-executable', false, load);
    expect(load).toHaveBeenCalledTimes(2);
  });
});
