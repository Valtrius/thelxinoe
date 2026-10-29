import type { Locator, Page } from '@playwright/test';

// Keeps a finger on the target until `opened` resolves.
export async function hold(
  page: Page,
  target: Locator,
  opened: () => Promise<void>,
) {
  await target.scrollIntoViewIfNeeded();
  const bounds = (await target.boundingBox())!;
  const touch = await page.context().newCDPSession(page);
  try {
    await touch.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [
        { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 },
      ],
    });
    await opened();
  } finally {
    await touch.send('Input.dispatchTouchEvent', {
      type: 'touchEnd',
      touchPoints: [],
    });
    await touch.detach();
  }
}
