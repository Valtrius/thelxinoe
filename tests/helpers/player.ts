import { expect, type Page } from '@playwright/test';

// The page keeps moving until the player has left, so a press that follows at
// once can miss its target.
export async function closePlayer(
  page: Page,
  press: 'click' | 'tap' = 'click',
) {
  const close = page.getByRole('button', { name: 'Close player', exact: true });
  await close[press]();
  await expect(page.getByRole('region', { name: 'Media player' })).toHaveCount(
    0,
  );
}
