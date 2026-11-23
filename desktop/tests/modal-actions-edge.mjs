// The hairline above a Modal's last row of actions (ui/parts.css, audit F17) follows the
// dialog's scroll: it tells that more content waits below. It is not motion, so it stays when the
// system asks for reduced motion, which turns every other animation off (styles.css).
// Run: node tests/modal-actions-edge.mjs (Vite preview on 127.0.0.1:1420). Synthetic sample data only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
try {
  for (const reducedMotion of ['no-preference', 'reduce']) {
    const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1280, height: 860 }, reducedMotion });
    await page.goto(base);
    await page.locator('.mail-row').first().waitFor();
    await page.getByRole('navigation', { name: 'Workspace' }).getByRole('button', { name: 'Contacts' }).click();
    await page.getByRole('button', { name: 'New contact' }).click();
    const dialog = page.getByRole('dialog', { name: 'New contact' });
    await expect(dialog).toBeVisible();
    const edge = await dialog.evaluate((el) => {
      const row = el.querySelector(':is(.modal-actions, .contact-actions, .google-dialog-actions, .google-setup-actions, .button-row):last-child');
      const style = row ? getComputedStyle(row) : null;
      return { name: style?.animationName, timeline: style?.animationTimeline };
    });
    assert.equal(edge.name, 'modal-actions-edge', `${reducedMotion}: the actions row keeps its scroll hairline (${JSON.stringify(edge)})`);
    assert.match(edge.timeline ?? '', /scroll/, `${reducedMotion}: driven by the dialog's scroll`);
    await page.close();
  }
  console.log('PASS the hairline above the actions follows the scroll, also with reduced motion');
} finally {
  await browser.close();
}
