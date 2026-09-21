import { test, expect, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('profile pictures are cropped, resized, saved and removable', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  let saved: string | null = null;
  await page.route('**/api/v1/me/avatar', async (route) => {
    saved = route.request().postDataJSON().image;
    await route.fulfill({ json: { avatar: saved } });
  });
  await page.goto('/');
  const data = await page.evaluate(() => {
    const canvas = document.createElement('canvas');
    canvas.width = 512;
    canvas.height = 256;
    const context = canvas.getContext('2d')!;
    context.fillStyle = '#ff0000';
    context.fillRect(0, 0, 256, 256);
    context.fillStyle = '#0000ff';
    context.fillRect(256, 0, 256, 256);
    return canvas.toDataURL('image/png').split(',')[1];
  });
  await page.getByLabel('Choose a picture').setInputFiles({
    name: 'two-colors.png',
    mimeType: 'image/png',
    buffer: Buffer.from(data, 'base64'),
  });
  const preview = page.getByRole('button', { name: 'Reframe profile picture' });
  await expect(preview).toBeVisible();
  await page
    .getByLabel('Picture zoom')
    .evaluate((element: HTMLInputElement) => {
      element.value = '2';
      element.dispatchEvent(new Event('input', { bubbles: true }));
    });
  await preview.scrollIntoViewIfNeeded();
  const bounds = (await preview.boundingBox())!;
  await page.mouse.move(bounds.x + 128, bounds.y + 128);
  await page.mouse.down();
  await page.mouse.move(bounds.x - 150, bounds.y + 128, { steps: 5 });
  await page.mouse.up();
  await page.getByLabel('Saved picture size').selectOption('128');
  await page.getByRole('button', { name: 'Save picture', exact: true }).click();
  await expect(
    page.getByRole('status').filter({ hasText: 'Profile picture saved' }),
  ).toBeVisible();
  const crop = await page.evaluate(async (source) => {
    const image = new Image();
    image.src = source!;
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 128;
    const context = canvas.getContext('2d')!;
    context.drawImage(image, 0, 0);
    return {
      width: image.naturalWidth,
      height: image.naturalHeight,
      pixel: [...context.getImageData(64, 64, 1, 1).data],
    };
  }, saved);
  expect(crop.width).toBe(128);
  expect(crop.height).toBe(128);
  expect(crop.pixel[2]).toBeGreaterThan(230);
  expect(crop.pixel[0]).toBeLessThan(20);
  await expect(page.locator('.sidebar-profile .avatar img')).toHaveAttribute(
    'src',
    saved!,
  );
  await page.getByRole('button', { name: 'Remove picture' }).click();
  await expect(page.locator('.sidebar-profile .avatar img')).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

async function expectFullHeightNavigation(page: Page) {
  await expect
    .poll(() =>
      page.locator('.settings-navigation').evaluate((nav) => {
        const workspace = nav
          .closest('.workspace-scroll')!
          .getBoundingClientRect();
        const bounds = nav.getBoundingClientRect();
        return Math.max(
          Math.abs(bounds.top - workspace.top),
          Math.abs(bounds.bottom - workspace.bottom),
        );
      }),
    )
    .toBeLessThanOrEqual(1);
}

test('short settings navigation fills the workspace and follows resizing', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true }),
  ).toBeVisible();
  await expectFullHeightNavigation(page);
  const panels = await page.locator('.settings-panels').evaluate((element) => ({
    width: element.getBoundingClientRect().width,
    inset:
      element.getBoundingClientRect().left -
      element.parentElement!.getBoundingClientRect().left,
  }));
  expect(panels.width).toBeLessThanOrEqual(880);
  expect(panels.inset).toBe(24);
  await expect(
    page.getByText('Use the server default or choose your own timezone.', {
      exact: false,
    }),
  ).toHaveCSS('line-height', '19.8px');
  await expect(page.locator('.settings-panels > .panel').first()).toHaveCSS(
    'padding',
    '0px 0px 24px',
  );
  for (const viewport of [
    { width: 960, height: 800 },
    { width: 1440, height: 1200 },
  ]) {
    await page.setViewportSize(viewport);
    await expectFullHeightNavigation(page);
    await expect(page.locator('.settings-navigation')).toHaveCSS(
      'width',
      viewport.width <= 1000 ? '155px' : '190px',
    );
  }
  await page.getByRole('button', { name: 'Dark theme', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expectFullHeightNavigation(page);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('long settings content scrolls while the menu stays full height', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { settingsSection: 'devices' });
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Your devices', exact: true }),
  ).toBeVisible();
  await page.locator('.workspace-scroll').evaluate((element) => {
    element.scrollTop = 450;
  });
  await expect
    .poll(() =>
      page
        .locator('.workspace-scroll')
        .evaluate((element) => element.scrollTop),
    )
    .toBe(450);
  await expectFullHeightNavigation(page);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('long admin navigation scrolls independently and keeps its last item reachable', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 560 });
  const fixture = await installUiFixture(page, { role: 'admin' });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  const last = nav.getByRole('button', { name: 'Audit', exact: true });
  await last.scrollIntoViewIfNeeded();
  await expect(last).toBeInViewport();
  expect(await nav.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  expect(
    await page
      .locator('.workspace-scroll')
      .evaluate((element) => element.scrollTop),
  ).toBe(0);
  await expectFullHeightNavigation(page);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('mobile settings use a horizontal menu at the existing breakpoint', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { role: 'admin' });
  for (const width of [720, 480, 320]) {
    await page.setViewportSize({ width, height: 850 });
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'Settings navigation' });
    await expect(nav).toHaveCSS('flex-direction', 'row');
    const bounds = await nav.boundingBox();
    const workspace = await page.locator('.workspace-scroll').boundingBox();
    expect(bounds!.width).toBeCloseTo(workspace!.width, 0);
    expect(bounds!.height).toBeLessThan(100);
    await expect(page.locator('.settings-panels > .panel').first()).toHaveCSS(
      'padding',
      '16px',
    );
    await nav
      .getByRole('button', { name: 'Audit', exact: true })
      .scrollIntoViewIfNeeded();
    await expect(
      nav.getByRole('button', { name: 'Audit', exact: true }),
    ).toBeInViewport();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('shared controls preserve form submission, choices and switch behavior', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  await page.goto('/');
  await page
    .getByRole('combobox', { name: 'Display timezone', exact: true })
    .selectOption('Europe/Paris');
  await page.getByRole('button', { name: 'Save display preferences' }).click();
  await expect(
    page.getByText('Display preferences saved.', { exact: true }),
  ).toBeVisible();
  expect(fixture.writes).toContainEqual({
    path: '/me/preferences',
    method: 'PUT',
    body: { timezone: 'Europe/Paris' },
  });
  await page.getByRole('button', { name: 'Smaller media cards' }).click();
  await expect(page.getByText('7 columns', { exact: true })).toBeVisible();
  await page.getByText('Fade watched videos', { exact: true }).click();
  await expect(
    page.getByRole('switch', { name: 'Fade watched videos' }),
  ).not.toBeChecked();
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          write.path === '/me/appearance' &&
          JSON.stringify(write.body) === '{"fade_watched":false}',
      ),
    )
    .toBe(true);
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Playback', exact: true })
    .click();
  const choices = page.getByRole('group', { name: 'Intro skipping' });
  await choices.getByRole('button', { name: 'Ignore', exact: true }).click();
  await expect(
    choices.getByRole('button', { name: 'Ignore', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  expect(
    fixture.writes.filter((write) => write.path === '/me/segments'),
  ).toHaveLength(0);
  await page
    .getByRole('button', { name: 'Save skip preferences', exact: true })
    .click();
  await expect
    .poll(() => fixture.writes.filter((write) => write.path === '/me/segments'))
    .toHaveLength(1);
  expect(
    fixture.writes.find((write) => write.path === '/me/segments')?.body,
  ).toEqual({ Intro: 'Ignore', Recap: 'Ask', Credits: 'Ask', Preview: 'Ask' });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('library cards retain their aspect ratio, hover actions and Ctrl-wheel zoom', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Movies' });
  await page.goto('/');
  const card = page.locator('.library-tile').first();
  await expect(card).toBeVisible();
  await expect(card.locator('.tile-art')).toHaveCSS('aspect-ratio', '2 / 3');
  await card.hover();
  await expect(card.locator('.card-actions')).toHaveCSS('opacity', '1');
  const width = (await card.boundingBox())!.width;
  await page.keyboard.down('Control');
  await page.mouse.wheel(0, -100);
  await page.keyboard.up('Control');
  await expect
    .poll(async () => (await card.boundingBox())!.width)
    .toBeGreaterThan(width);
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          write.path === '/me/appearance' &&
          JSON.stringify(write.body) === '{"card_columns":5}',
      ),
    )
    .toBe(true);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('the sign-in button still submits the form by keyboard', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('fixture');
  await page.getByLabel('Password', { exact: true }).fill('fixture password');
  const button = page.getByRole('button', { name: 'Sign in', exact: true });
  expect(
    await button.evaluate(
      (element) =>
        element.getBoundingClientRect().width ===
        element.closest('form')!.getBoundingClientRect().width,
    ),
  ).toBe(true);
  await page.getByLabel('Password', { exact: true }).press('Enter');
  await expect(
    page.getByRole('navigation', { name: 'Settings navigation' }),
  ).toBeVisible();
  expect(fixture.writes).toContainEqual({
    path: '/auth/login',
    method: 'POST',
    body: { username: 'fixture', password: 'fixture password' },
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
