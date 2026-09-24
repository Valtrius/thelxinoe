import { describe, expect, it } from 'vitest';
import { PlaybackActivity } from './playback-activity';

describe('active playback time', () => {
  it('does not infer activity from seeks and bounds a suspended process', () => {
    let now = 0;
    const activity = new PlaybackActivity(() => now);
    activity.setActive(true);
    now = 5000;
    activity.setActive(false); // seek begins, regardless of target position
    now = 7000;
    activity.setActive(true); // decoding resumes
    now = 12_000;
    expect(activity.seconds()).toBe(10);
    now += 3600_000;
    expect(activity.seconds()).toBe(40);
  });
});
