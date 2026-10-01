import { chromium } from '@playwright/test';

export const browserArguments = ['--autoplay-policy=no-user-gesture-required'];
export const browserConnection = () =>
  process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT
    ? {
        wsEndpoint: process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT,
        exposeNetwork: '<loopback>',
        timeout: 30000,
        headers: {
          'x-playwright-launch-options': JSON.stringify({
            headless: true,
            args: browserArguments,
          }),
        },
      }
    : undefined;

export function launchBrowser() {
  const connection = browserConnection();
  if (!connection) return chromium.launch({ args: browserArguments });
  const { wsEndpoint, ...options } = connection;
  return chromium.connect(wsEndpoint, options);
}
