import type { Page } from '@playwright/test';

// Player taps are timed gestures. Freezing the page clock (installed with
// page.clock.install() before navigation) keeps latency between Playwright
// actions on a loaded CI runner from splitting or expiring a gesture; tests
// then advance time explicitly with page.clock.runFor().
export async function freezeClock(page: Page) {
  // pauseAt cannot move backwards, so stop slightly ahead of the page time.
  await page.clock.pauseAt((await page.evaluate(() => Date.now())) + 1000);
}
