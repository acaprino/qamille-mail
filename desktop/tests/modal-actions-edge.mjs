// The dialog footer is provided by Sigrid. On a short screen its actions must remain
// reachable and visible when navigating the overflowing form with the keyboard.
import { chromium, expect } from '@playwright/test';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true });
try {
  for (const reducedMotion of ['no-preference', 'reduce']) {
    const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1280, height: 720 }, reducedMotion });
    await page.goto(base);
    await page.getByRole('list', { name: 'Workspace' }).getByRole('button', { name: 'Contacts' }).click();
    const open = page.getByRole('button', { name: 'New contact', exact: true }).first();
    await open.click();
    const dialog = page.getByRole('dialog', { name: 'New contact' });
    await expect(dialog).toBeVisible();
    await page.setViewportSize({ width: 680, height: 380 });
    await expect.poll(() => dialog.evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
    const save = dialog.getByRole('button', { name: 'Save', exact: true });
    for (let n = 0; n < 60; n++) {
      await page.keyboard.press('Tab');
      if (await save.evaluate(el => el === document.activeElement)) break;
    }
    await expect(save).toBeFocused();
    await expect(save).toBeInViewport({ ratio: 0.95 });
    await page.keyboard.press('Shift+Tab');
    const cancel = dialog.getByRole('button', { name: 'Cancel', exact: true });
    await expect(cancel).toBeFocused();
    await expect(cancel).toBeInViewport({ ratio: 0.95 });
    await page.keyboard.press('Enter');
    await expect(dialog).toHaveCount(0);
    await page.close();
  }
} finally {
  await browser.close();
}
