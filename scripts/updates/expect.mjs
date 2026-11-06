import { expect as playwrightExpect } from '@playwright/test';
import { budgets } from '../ci-readiness.mjs';

// A loaded runner can stall a page for longer than Playwright's 5 s default.
export const expect = playwrightExpect.configure({ timeout: budgets.request });
