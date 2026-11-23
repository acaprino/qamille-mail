// Button primitive (spec 2026-11-08 ark-compatto §8, Task 3): the optional tip and shortcut, which
// make the button the trigger of a Tooltip, and the Compatto measures (height from --control-md,
// radius from --radius-md). The primitive is mounted alone through tests/support/mount.mjs.
// Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { mountGallery } from './support/mount.mjs';

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
const watch = (page) => page.on('pageerror', (error) => errors.push(error.message));
try {
  const context = await browser.newContext({ locale: 'en-US' });
  const page = await context.newPage();
  watch(page);
  await mountGallery(page, 'button');

  const archive = page.getByRole('button', { name: 'Archive', exact: true });
  const tooltip = page.locator('[data-slot="tooltip-content"]');
  const send = page.getByRole('button', { name: 'Send', exact: true });

  // (a) A tip is a hint, not the name: no title attribute, the aria-label stays the accessible name.
  assert.equal(await archive.getAttribute('title'), null);
  assert.equal(await archive.getAttribute('aria-label'), 'Archive');
  assert.equal(await page.locator('button[title]').count(), 0);
  await expect(tooltip).toHaveCount(0);
  console.log('PASS A button with a tip has no title attribute and keeps its aria-label');

  // (b) Keyboard focus shows the tip at once, with the text and the shortcut in a <kbd>.
  for (let i = 0; i < 5 && !(await archive.evaluate((el) => el === document.activeElement)); i++) await page.keyboard.press('Tab');
  await expect(archive).toBeFocused();
  await expect(tooltip).toBeVisible();
  await expect(tooltip).toContainText('Archive');
  await expect(tooltip.locator('[data-slot="tooltip-shortcut"]')).toHaveText('E');
  console.log('PASS Keyboard focus opens a tooltip with the text and the shortcut');

  // (c) Escape closes it; the focus stays on the button.
  await page.keyboard.press('Escape');
  await expect(tooltip).toBeHidden();
  await expect(archive).toBeFocused();
  console.log('PASS Escape closes the tooltip');

  // Leaving the button closes it too: Tab moves on and the tooltip goes.
  await page.keyboard.press('Tab');
  await expect(send).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(archive).toBeFocused();
  await expect(tooltip).toBeVisible();
  await page.keyboard.press('Tab');
  await expect(send).toBeFocused();
  await expect(tooltip).toBeHidden();
  console.log('PASS Moving the focus away closes the tooltip');

  // Hover opens it after a short delay (about 400 ms), the pointer leaving closes it.
  const hoverAt = Date.now();
  await archive.hover();
  await expect(tooltip).toBeVisible({ timeout: 3000 });
  const waited = Date.now() - hoverAt;
  assert.ok(waited >= 300, `the tooltip opened after ${waited} ms: hover has a delay`);
  await expect(tooltip).toContainText('Archive');
  await page.mouse.move(400, 400);
  await expect(tooltip).toBeHidden();
  console.log('PASS Hover opens the tooltip after a delay and leaving closes it');

  // The button still does its job, and pressing it hides the hint.
  await archive.hover();
  await expect(tooltip).toBeVisible({ timeout: 3000 });
  await archive.click();
  assert.equal(await page.locator('#archived').textContent(), '1', "the caller's onclick still runs");
  await expect(tooltip).toBeHidden();
  console.log('PASS The caller’s onclick still runs and a press closes the tooltip');

  // A button without a tip is exactly the old button: no Ark attribute, no wrapper, no description.
  for (const name of ['Send', 'Cancel', 'Later', 'Delete']) {
    const plain = page.getByRole('button', { name, exact: true });
    assert.equal(await plain.getAttribute('data-scope'), null, `${name}: no tooltip trigger attributes`);
    assert.equal(await plain.getAttribute('aria-describedby'), null);
    assert.equal(await plain.evaluate((el) => el.parentElement.id), 'plain-row', `${name}: a direct child of its row, no wrapper`);
  }
  console.log('PASS A button without a tip renders no tooltip machinery');

  // (d) Compatto measures, in computed value: 32 px high, radius-md 6 px; the icon button is 32 x 32.
  const measure = (locator) => locator.evaluate((el) => {
    const style = getComputedStyle(el);
    return { height: style.height, width: style.width, radius: style.borderTopLeftRadius };
  });
  const primary = await measure(send);
  assert.equal(primary.height, '32px');
  assert.equal(primary.radius, '6px');
  const secondary = await measure(page.getByRole('button', { name: 'Cancel', exact: true }));
  assert.equal(secondary.height, '32px');
  assert.equal(secondary.radius, '6px');
  const icon = await measure(archive);
  assert.deepEqual([icon.width, icon.height], ['32px', '32px']);
  const plainIcon = await measure(page.getByRole('button', { name: 'Delete', exact: true }));
  assert.deepEqual([plainIcon.width, plainIcon.height], ['32px', '32px']);
  console.log('PASS Buttons are 32px high with a 6px radius, the icon button 32 by 32');

  // No tooltip on touch: a tap shows none, however long it waits.
  const touch = await browser.newContext({ locale: 'en-US', hasTouch: true });
  const touchPage = await touch.newPage();
  watch(touchPage);
  await mountGallery(touchPage, 'button');
  await touchPage.getByRole('button', { name: 'Archive', exact: true }).tap();
  await touchPage.waitForTimeout(800);
  await expect(touchPage.locator('[data-slot="tooltip-content"]')).toHaveCount(0);
  assert.equal(await touchPage.locator('#archived').textContent(), '1', 'the tap still presses the button');
  console.log('PASS A touch tap opens no tooltip');

  // (e) Icon: an unknown name silently falls back to the mail drawing, so each new name must really
  // have its own path. Each one renders markup different from mail's and from every other, and
  // draws something inside the 24 x 24 box (an empty or malformed `d` has a zero bounding box).
  const fresh = ['copy', 'pause', 'stop', 'video', 'alert', 'play'];
  const iconPage = await context.newPage();
  watch(iconPage);
  await mountGallery(iconPage, 'icons', { names: ['mail', ...fresh, 'no-such-icon'] });
  const drawn = await iconPage.evaluate(() =>
    Object.fromEntries(
      [...document.querySelectorAll('[data-icon]')].map((holder) => {
        const svg = holder.querySelector('svg');
        const box = svg.getBBox();
        return [holder.dataset.icon, { markup: svg.innerHTML, width: box.width, height: box.height, right: box.x + box.width, bottom: box.y + box.height, left: box.x, top: box.y }];
      }),
    ),
  );
  assert.equal(drawn['no-such-icon'].markup, drawn.mail.markup, 'an unknown name is the mail fallback (the check below is meaningful)');
  for (const name of fresh) {
    assert.notEqual(drawn[name].markup, drawn.mail.markup, `${name}: is not the mail fallback`);
    assert.ok(drawn[name].width > 4 && drawn[name].height > 4, `${name}: draws a shape (${drawn[name].width} x ${drawn[name].height})`);
    assert.ok(drawn[name].left >= 0 && drawn[name].top >= 0 && drawn[name].right <= 24 && drawn[name].bottom <= 24, `${name}: stays in the 24 x 24 box`);
  }
  assert.equal(new Set(fresh.map((name) => drawn[name].markup)).size, fresh.length, 'the six new icons are all different');
  console.log('PASS copy, pause, stop, video, alert and play are their own drawings, not the mail fallback');

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
