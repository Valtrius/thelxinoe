import { describe, expect, it, vi } from 'vitest';
import { loadMpvSchema } from './mpv-schema';

describe('MPV option cache', () => {
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
