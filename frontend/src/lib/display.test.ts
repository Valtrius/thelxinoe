import { afterEach, describe, expect, it } from 'vitest';
import {
  display,
  formatTime,
  formatTimestamp,
  resetDisplay,
} from './display.svelte';

// 2026-01-15 13:05:09 UTC
const instant = 1768482309;

afterEach(resetDisplay);

describe('display formatting', () => {
  it("uses the signed-in user's zone and clock", () => {
    display.timezone = 'Asia/Tokyo';
    display.timeFormat = '12h';
    expect(formatTimestamp(instant)).toMatch(/1\/15\/2026.*10:05:09.*PM/);
    expect(formatTime(instant)).toMatch(/10:05.*PM/);
    display.timeFormat = '24h';
    expect(formatTime(instant)).toBe('22:05');
  });

  it('shows server schedules in the zone they are given', () => {
    display.timezone = 'Asia/Tokyo';
    expect(formatTimestamp(instant, 'UTC')).toMatch(/13:05:09/);
    expect(formatTime(instant, 'America/New_York')).toBe('08:05');
  });
});
