import { chromium } from '@playwright/test';

export function launchBrowser() {
  const endpoint = process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT;
  return endpoint
    ? chromium.connect(endpoint, {
        exposeNetwork: '<loopback>',
        timeout: 30000,
      })
    : chromium.launch();
}
