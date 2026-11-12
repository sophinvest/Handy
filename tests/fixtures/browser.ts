import { test as base, expect } from "@playwright/test";

// Reuse a caller-owned, resource-guarded browser when running in shared CI.
// Normal developer runs retain Playwright's default browser and launch settings.
export const test = process.env.HANDY_TEST_CDP
  ? base.extend({
      browser: async ({ playwright }, use) => {
        const browser = await playwright.chromium.connectOverCDP(
          process.env.HANDY_TEST_CDP!,
        );
        await use(browser);
        await browser.close();
      },
    })
  : base;
export { expect };
