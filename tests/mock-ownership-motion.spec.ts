import { expect, test, type Page, type TestInfo } from '@playwright/test';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});

async function prepare(page: Page, preset = 'review', delay = 0) {
  await page.request.post('/__mock/services', {
    data: { action: 'scenario', kind: 'seerr', preset },
  });
  const state = await (await page.request.get('/__mock/services')).json();
  await page.request.post('/__mock/services', {
    data: {
      action: 'settings',
      kind: 'seerr',
      ...state.services.seerr,
      paused: true,
      delay,
    },
  });
  await page.goto('/#admin/services/seerr');
}

async function capture(page: Page, info: TestInfo, name: string) {
  await info.attach(name, {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
}

test('slow buttons replace their labels with spinners and selection follows new backend steps', async ({
  page,
}, info) => {
  await prepare(page, 'review', 3000);
  await page
    .getByRole('button', { name: 'Continue ownership review', exact: true })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Take ownership of Seerr' });
  const nav = dialog.getByRole('navigation', {
    name: 'Ownership transfer steps',
  });
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  const project = dialog.getByRole('button', {
    name: "Retire the whole project's automation",
  });
  const width = (await project.boundingBox())!.width;
  await project.click();
  await expect(project).toHaveAttribute('aria-busy', 'true');
  await expect(project.locator('svg.animate-spin')).toHaveCount(1);
  expect(
    await project
      .locator('.opacity-0')
      .evaluate((node) => getComputedStyle(node).opacity),
  ).toBe('0');
  expect((await project.boundingBox())!.width).toBeCloseTo(width, 0);
  await expect(
    dialog.getByRole('button', { name: 'Detach only this service' }),
  ).toBeDisabled();
  await capture(page, info, 'compose-choice-pending');
  await expect(project).toBeEnabled();
  for (const toggle of await dialog.getByRole('switch').all())
    await toggle.check();
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await dialog
    .getByRole('switch', {
      name: 'I reviewed every change, and I understand the original stops',
    })
    .check();
  const approve = dialog.getByRole('button', {
    name: 'Take ownership',
    exact: true,
  });
  await approve.click();
  await expect(approve).toHaveAttribute('aria-busy', 'true');
  await expect(approve.locator('svg.animate-spin')).toHaveCount(1);
  await capture(page, info, 'ownership-approval-pending');
  await expect(
    nav.getByRole('button', { name: /^5\. Stop the original/ }),
  ).toHaveAttribute('aria-current', 'step');
  await expect(
    dialog.getByRole('heading', { name: 'Stop the original', exact: true }),
  ).toBeVisible();
  await page.request.post('/__mock/services', {
    data: { action: 'advance', kind: 'seerr' },
  });
  await expect(
    nav.getByRole('button', { name: /^6\. Copy appdata/ }),
  ).toHaveAttribute('aria-current', 'step');
  await nav.getByRole('button', { name: /^2\. Review changes/ }).click();
  const refresh = page.waitForResponse((response) =>
    response.url().endsWith('/admin/transfers/transfer-seerr'),
  );
  await refresh;
  await expect(
    nav.getByRole('button', { name: /^2\. Review changes/ }),
  ).toHaveAttribute('aria-current', 'step');
  await page.request.post('/__mock/services', {
    data: { action: 'advance', kind: 'seerr' },
  });
  await expect(
    nav.getByRole('button', { name: /^7\. Validate a disposable copy/ }),
  ).toHaveAttribute('aria-current', 'step');
  await capture(page, info, 'following-backend-progress');
  for (let i = 0; i < 5; i++)
    await page.request.post('/__mock/services', {
      data: { action: 'advance', kind: 'seerr' },
    });
  await expect(
    nav.getByRole('button', { name: /^12\. Complete the transfer/ }),
  ).toHaveAttribute('aria-current', 'step');
  const remove = dialog.getByRole('button', {
    name: 'Remove the stopped original container',
  });
  await remove.click();
  await expect(remove).toHaveAttribute('aria-busy', 'true');
  await capture(page, info, 'original-removal-pending');
  await expect(remove).toHaveCount(0);
});

test('ownership motion respects reduced motion and keeps the mobile dialog within the viewport', async ({
  page,
}, info) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 390, height: 844 });
  await prepare(page);
  await page
    .getByRole('button', { name: 'Continue ownership review', exact: true })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Take ownership of Seerr' });
  await expect(
    dialog.getByRole('region', { name: 'Deployment changes' }),
  ).toBeVisible();
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  const nav = dialog.getByRole('navigation', {
    name: 'Ownership transfer steps',
  });
  const selected = nav.locator('[aria-current="step"]');
  await expect(selected).toHaveAccessibleName(/^3\. Confirm automation/);
  expect(
    await dialog.evaluate(
      (node) =>
        node
          .getAnimations({ subtree: true })
          .filter(
            (animation) =>
              animation.effect!.getComputedTiming().iterations !== Infinity,
          ).length,
    ),
  ).toBe(0);
  expect(
    (await nav.locator('[data-transfer-highlight]').boundingBox())!.y,
  ).toBeCloseTo((await selected.boundingBox())!.y, 0);
  expect((await dialog.boundingBox())!.height).toBeLessThanOrEqual(844);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await capture(page, info, 'mobile-reduced-motion');
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole('button', {
      name: 'Continue ownership review',
      exact: true,
    }),
  ).toBeFocused();
});

test('mobile steps show three rows, allow manual scrolling and scroll to backend selections', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await prepare(page, 'copy');
  await page
    .getByRole('button', { name: 'Open ownership transfer', exact: true })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Take ownership of Seerr' });
  const nav = dialog.getByRole('navigation', {
    name: 'Ownership transfer steps',
  });
  const visibleRows = () =>
    nav.evaluate((node) => {
      const bounds = node.getBoundingClientRect();
      return [...node.querySelectorAll('button')].filter((button) => {
        const row = button.getBoundingClientRect();
        return row.top >= bounds.top - 0.5 && row.bottom <= bounds.bottom + 0.5;
      }).length;
    });
  await expect(
    nav.getByRole('button', { name: /^6\. Copy appdata/ }),
  ).toHaveAttribute('aria-current', 'step');
  await expect.poll(visibleRows).toBe(3);
  expect((await nav.boundingBox())!.height).toBeLessThan(180);
  await nav.evaluate((node) => node.scrollTo({ top: 0, behavior: 'instant' }));
  const polled = page.waitForResponse((response) =>
    response.url().endsWith('/admin/transfers/transfer-seerr'),
  );
  await polled;
  expect(await nav.evaluate((node) => node.scrollTop)).toBe(0);
  await nav.evaluate((node) => {
    const samples: number[] = [];
    (window as unknown as { stepScrollSamples: number[] }).stepScrollSamples =
      samples;
    const start = performance.now();
    const sample = () => {
      samples.push(node.scrollTop);
      if (performance.now() - start < 4500) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
  await page.request.post('/__mock/services', {
    data: { action: 'advance', kind: 'seerr' },
  });
  await expect(
    nav.getByRole('button', { name: /^7\. Validate a disposable copy/ }),
  ).toHaveAttribute('aria-current', 'step');
  await expect
    .poll(() =>
      nav.evaluate((node) => {
        const bounds = node.getBoundingClientRect();
        const selected = node
          .querySelector('[aria-current="step"]')!
          .getBoundingClientRect();
        return Math.abs(
          selected.top + selected.height / 2 - bounds.top - bounds.height / 2,
        );
      }),
    )
    .toBeLessThan(1);
  const samples = await page.evaluate(
    () =>
      (window as unknown as { stepScrollSamples: number[] }).stepScrollSamples,
  );
  expect(samples.some((value) => value > 0 && value < samples.at(-1)!)).toBe(
    true,
  );
  await expect.poll(visibleRows).toBe(3);
  await capture(page, info, 'mobile-selected-step-scrolled');
  for (let i = 0; i < 5; i++)
    await page.request.post('/__mock/services', {
      data: { action: 'advance', kind: 'seerr' },
    });
  const complete = nav.getByRole('button', {
    name: /^12\. Complete the transfer/,
  });
  await expect(complete).toHaveAttribute('aria-current', 'step');
  await expect
    .poll(() =>
      nav.evaluate((node) => {
        const bounds = node.getBoundingClientRect();
        const selected = node
          .querySelector('[aria-current="step"]')!
          .getBoundingClientRect();
        return (
          selected.top >= bounds.top - 1 && selected.bottom <= bounds.bottom + 1
        );
      }),
    )
    .toBe(true);
  await capture(page, info, 'mobile-completed-step-visible');
  await info.attach('mobile-step-scroll', {
    body: JSON.stringify(samples),
    contentType: 'application/json',
  });
});

for (const [width, height] of [
  [390, 844],
  [667, 375],
]) {
  test(`mobile ownership keeps its title and steps pinned while content scrolls at ${width}x${height}`, async ({
    page,
  }, info) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.setViewportSize({ width, height });
    await prepare(page);
    await page
      .getByRole('button', { name: 'Continue ownership review', exact: true })
      .click();
    const dialog = page.getByRole('dialog', {
      name: 'Take ownership of Seerr',
    });
    const title = dialog.getByRole('heading', {
      name: 'Take ownership of Seerr',
      exact: true,
    });
    const nav = dialog.getByRole('navigation', {
      name: 'Ownership transfer steps',
    });
    const body = dialog.getByRole('region', {
      name: 'Review changes',
      exact: true,
    });
    await expect(body).toBeVisible();
    const titleBefore = await title.boundingBox();
    const navBefore = await nav.boundingBox();
    await body.evaluate((node) =>
      node.scrollTo({ top: node.scrollHeight, behavior: 'instant' }),
    );
    expect(await body.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
    expect(await dialog.evaluate((node) => node.scrollTop)).toBe(0);
    expect(await title.boundingBox()).toEqual(titleBefore);
    expect(await nav.boundingBox()).toEqual(navBefore);
    expect((await body.boundingBox())!.y).toBeGreaterThanOrEqual(
      navBefore!.y + navBefore!.height,
    );
    expect((await dialog.boundingBox())!.height).toBeLessThanOrEqual(height);
    await capture(page, info, `pinned-mobile-steps-${width}x${height}`);
    await body.getByRole('button', { name: 'Continue', exact: true }).click();
    const automation = dialog.getByRole('region', {
      name: 'Confirm automation is retired',
      exact: true,
    });
    await expect(automation).toBeVisible();
    expect(await automation.evaluate((node) => node.scrollTop)).toBe(0);
    expect(await title.boundingBox()).toEqual(titleBefore);
    expect(await nav.boundingBox()).toEqual(navBefore);
  });
}
