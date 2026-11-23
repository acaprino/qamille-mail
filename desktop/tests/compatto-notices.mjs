// Task 17 of the Ark Compatto plan: notices, undo, activity and the accessory windows, measured in
// computed values and driven through roles and accessible names. Sample data only.
// Run: node scripts/test-browser.mjs compatto-notices
import { expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import fs from 'node:fs/promises';
import path from 'node:path';
import { box, css, openApp, previewUrl } from './support/compatto.mjs';
import { mountGallery } from './support/mount.mjs';

const root = path.resolve(import.meta.dirname, '..');
const token = (page, name, property = 'color') =>
  page.evaluate(([value, prop]) => {
    const probe = document.createElement('i');
    probe.style.setProperty(prop, `var(${value})`);
    document.body.append(probe);
    const resolved = getComputedStyle(probe).getPropertyValue(prop);
    probe.remove();
    return resolved;
  }, [name, property]);

const { browser, page, errors } = await openApp();
page.setDefaultTimeout(15000);
const sync = page.getByRole('button', { name: 'Sync mail', exact: true });
// Ark writes data-scope and data-part; the old class stays as the hook the sync tests use.
const toast = page.locator('[data-scope="toast"][data-part="root"]');

try {
  // (a) A notice: Sigrid's Toasts, in the region «Notices», with its text and a close button.
  await sync.click();
  await expect(toast).toHaveCount(1);
  await expect(page.getByRole('status', { name: 'Notices' }).locator('[data-scope="toast"][data-part="root"]')).toHaveCount(1);
  await expect(toast).toContainText('Your inbox is up to date');
  const close = toast.getByRole('button', { name: /^Close the notice/ });
  await expect(close).toBeVisible();
  console.log('PASS a notice: the region «Notices», its text and a close button');

  // (b) The close button dismisses it.
  await close.click();
  await expect(toast).toHaveCount(0);
  console.log('PASS the close button dismisses the notice');

  // (c) It closes by itself (10 s for a notice that is not an error).
  await sync.click();
  await expect(toast).toHaveCount(1);
  await expect(toast).toHaveCount(0, { timeout: 14000 });
  console.log('PASS a notice closes by itself');

  // (d) An error stays until it is closed; the next notice comes after it.
  await page.evaluate(() => (window.__previewCommandFail = { sync_mail: 'Synthetic failure' }));
  await sync.click();
  await expect(toast).toContainText('Synthetic failure');
  await page.evaluate(() => (window.__previewCommandFail = {}));
  await sync.click();
  await expect(toast.last()).toContainText('Your inbox is up to date');
  await expect(toast.first()).toContainText('Synthetic failure');
  for (const dismiss of await toast.getByRole('button', { name: /^Close the notice/ }).all()) await dismiss.click();
  await expect(toast).toHaveCount(0);
  console.log('PASS an error stays; a new notice follows it');

  // (f) With a composer open the notice lifts above the dock.
  await page.getByRole('button', { name: /^New message/ }).click();
  await expect(page.locator('.app-shell.has-composers')).toHaveCount(1);
  await sync.click();
  await expect(toast).toHaveCount(1);
  await expect(page.locator('.composer-dock')).toBeVisible();
  const dock = await box(page.locator('.composer-dock'));
  assert.ok((await box(toast)).y + (await box(toast)).height <= dock.y, 'the notice is above the dock');
  console.log('PASS the notice lifts above the composer dock');

  // (g) The reminder window uses Plex, not Lexend (ruling R8).
  const css8 = await fs.readFile(path.join(root, 'src/reminder/reminder.css'), 'utf8');
  assert.ok(!/lexend/i.test(css8), 'reminder.css has no Lexend');
  assert.match(css8, /fonts.css/, "reminder.css imports the Plex files of Sigrid");
  assert.match(css8, /font-family:\s*var\(--font-sans\)/, 'the body uses the font token');
  const reminder = await browser.newPage();
  await reminder.goto(`${previewUrl}/reminder.html`);
  await reminder.evaluate(() => document.fonts.ready);
  const faces = await reminder.evaluate(() => [...document.fonts].map((f) => f.family.replaceAll('"', '')));
  assert.ok(!faces.includes('Lexend'), `no Lexend face: ${faces}`);
  assert.ok(faces.includes('IBM Plex Sans'), `Plex face present: ${faces}`);
  assert.match(await reminder.evaluate(() => getComputedStyle(document.body).fontFamily), /IBM Plex Sans/);
  await reminder.close();
  console.log('PASS the reminder window loads Plex and has no Lexend');

  // (h) Undo notice: keeps its structure and timings.
  await page.reload();
  await page.locator('.compose-main').waitFor();
  await page.getByRole('button', { name: 'Done', exact: true }).click();
  const notices = page.getByRole('region', { name: 'Recent email actions' });
  const notice = notices.locator('.undo-notice');
  await expect(notice).toHaveCount(1);
  const undoButton = notice.getByRole('button', { name: /^Undo:/ });
  await expect(undoButton).toBeVisible();
  await expect(notices.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '5');
  console.log('PASS undo notice: the Undo button and the countdown of Sigrid\u2019s Progress');

  // (i) Activity card in the sidebar footer: it shows the line, and a click opens the panel with its rows.
  await page.evaluate(() => window.__previewAnnounce({ kind: 'sync-progress', account_id: 2, phase: 'syncing', added: 0 }));
  const line = page.locator('.sidebar .activity-indicator .activity-line');
  await expect(line).toBeVisible();
  await line.click();
  const row = page.locator('.activity-panel .activity-row').first();
  await expect(row).toBeVisible();
  await page.evaluate(() => window.__previewAnnounce({ kind: 'sync-progress', account_id: 2, phase: 'complete', added: 0 }));
  console.log('PASS activity card: the line and the panel with its rows');

  // (j) Window controls: Windows-width caption buttons on tokens, hint through Tooltip, no title.
  const bar = await browser.newPage({ locale: 'en-US', viewport: { width: 600, height: 300 } });
  await bar.addInitScript(() => {
    window.isTauri = true;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    let id = 0;
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
      transformCallback: () => ++id,
      invoke: async (command) => (command === 'plugin:window|is_maximized' ? false : command.startsWith('plugin:event|listen') ? 1 : null),
    };
  });
  await mountGallery(bar, 'window-controls');
  const controls = bar.getByRole('group', { name: 'Window controls' });
  for (const name of ['Minimize window', 'Maximize window', 'Close window']) {
    const button = controls.getByRole('button', { name, exact: true });
    const size = await box(button);
    assert.equal(Math.round(size.width), 46, `${name} keeps the caption-button width`);
    assert.equal(Math.round(size.height), 40, `${name} fills the bar`);
    assert.equal(await button.getAttribute('title'), null, `${name} has no title attribute`);
    assert.equal(await css(button, 'border-top-left-radius'), '0px');
  }
  const closeButton = controls.getByRole('button', { name: 'Close window', exact: true });
  await closeButton.hover();
  assert.equal(await css(closeButton, 'background-color'), await token(bar, '--window-close', 'background-color'), 'close turns red');
  await bar.keyboard.press('Tab');
  await bar.keyboard.press('Tab');
  await expect(bar.getByRole('tooltip')).toBeVisible();
  await bar.close();
  console.log('PASS window controls: 46px buttons, tokens, hint through Tooltip, no title');

  // (k) The reminder chip: 52px high as the shell expects, 32px controls, Plex.
  const chipPage = await browser.newPage({ locale: 'en-US', viewport: { width: 700, height: 200 } });
  await chipPage.addInitScript(() => {
    window.isTauri = true;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    let id = 0;
    const view = { chips: [{ key: 'a', title: 'Design review', start: 1800000000, end: 1800003600, relative: { kind: 'in', minutes: 5 } }], recording: false, dark: false, locale: 'en-US', time_zone: 'UTC' };
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'reminder' }, currentWebview: { label: 'reminder' } },
      transformCallback: () => ++id,
      invoke: async (command) => (command === 'reminder_view' ? view : command.startsWith('plugin:event|listen') ? 1 : null),
    };
  });
  await chipPage.goto(`${previewUrl}/reminder.html`);
  const chip = chipPage.getByRole('region', { name: 'Reminder: Design review' });
  await expect(chip).toBeVisible();
  assert.equal(Math.round((await box(chip)).height), 52, 'the chip keeps the 52px the window sizing relies on');
  assert.equal(await css(chip, 'border-top-left-radius'), '12px', 'radius-xl');
  assert.match(await css(chip, 'font-family'), /IBM Plex Sans/);
  for (const name of ['Join', 'Join and record', 'Close']) {
    const button = chip.getByRole('button', { name, exact: true });
    assert.equal(Math.round((await box(button)).height), 32, `${name} is a 32px control`);
    assert.equal(await css(button, 'border-top-left-radius'), '6px', `${name} radius-md`);
  }
  assert.equal(await css(chip.getByRole('button', { name: 'Join', exact: true }), 'background-color'), await token(chipPage, '--color-primary', 'background-color'));
  await chipPage.close();
  console.log('PASS the reminder chip: 52px, 32px controls, radius tokens, Plex');

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
