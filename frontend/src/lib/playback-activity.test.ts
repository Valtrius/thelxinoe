import { describe, expect, it } from 'vitest';
import { PlaybackActivity } from './playback-activity';

describe('active playback time', () => {
  it('counts active intervals, preserving partial seconds across pause and buffering', () => {
    let now = 0;
    const activity = new PlaybackActivity(() => now);
    now = 2000;
    expect(activity.seconds()).toBe(0); // prepared media is idle
    expect(activity.setActive(true)).toBe(true);
    now = 2250;
    activity.setActive(false);
    now = 20_000;
    expect(activity.seconds()).toBe(0.25);
    activity.setActive(true);
    now = 20_750;
    expect(activity.seconds()).toBe(1);
    expect(activity.seconds()).toBe(1); // reading/retrying never doubles a sample
    activity.setActive(false);
    now = 30_000;
    expect(activity.seconds()).toBe(1);
  });

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
