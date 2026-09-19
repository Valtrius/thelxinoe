import { chromium, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
const code = readFileSync('.local/tv-ui.xml', 'utf8').match(
  /text="(\d{3}) (\d{3})"/,
);
if (!code)
  throw new Error(
    'Open Quick Connect on the test TV and dump its UI before running this test.',
  );
const browser = await chromium.launch();
try {
  const context = await browser.newContext({ ignoreHTTPSErrors: true });
  const page = await context.newPage();
  await page.goto('https://localhost:21443');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByLabel('TV code', { exact: true }).fill(code[1] + code[2]);
  await page.getByRole('button', { name: 'Find device', exact: true }).click();
  const lookup = page.getByRole('region', {
    name: 'Quick Connect',
    exact: true,
  });
  await expect
    .poll(
      async () =>
        (await lookup
          .getByRole('button', { name: 'Connect this device', exact: true })
          .count()) + (await lookup.getByRole('alert').count()),
    )
    .toBeGreaterThan(0);
  if (await lookup.getByRole('alert').count())
    throw new Error(await lookup.getByRole('alert').innerText());
  await expect(
    page.getByRole('button', { name: 'Connect this device', exact: true }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Connect this device', exact: true })
    .click();
  await expect(
    page.getByText('Device approved. Continue on your TV.', { exact: true }),
  ).toBeVisible();
  await page
    .getByRole('region', { name: 'Quick Connect', exact: true })
    .screenshot({ path: '.local/tv-quick-connect.png' });
  console.log(
    'Approved the emulator device through the real web Quick Connect flow.',
  );
} finally {
  await browser.close();
}
