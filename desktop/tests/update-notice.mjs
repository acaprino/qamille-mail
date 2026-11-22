// The notice of an update ready to install (UpdateNotice.tsx, src-tauri/src/updates.rs), on a simulated
// desktop bridge: it shows what update_status or update-available says and stays there, its button calls
// update_install (which quits through the close guards), and a failure reaches the app's error notice.
// Outside the desktop it asks nothing and shows nothing.
// Run against the browser preview: QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const shots = process.env.UPDATE_NOTICE_SHOTS;
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const errors = [];
const ready = { version: '1.2.0', notes: 'Faster search.\nSigned installer.' };

async function open({ desktop = true, status = null, install = 'ok', query = '' } = {}) {
  const context = await browser.newContext({
    viewport: { width: 1200, height: 800 },
    locale: 'en-US',
  });
  await context.addInitScript(
    ({ desktop, status, install }) => {
      window.calls = [];
      if (!desktop) return;
      window.isTauri = true;
      const callbacks = {};
      let next = 0;
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      window.__TAURI_INTERNALS__ = {
        transformCallback(callback) {
          next += 1;
          callbacks[next] = callback;
          return next;
        },
        async invoke(command, args = {}) {
          window.calls.push(command);
          if (command === 'update_status') return status;
          if (command === 'plugin:event|listen') {
            if (args.event === 'update-available')
              window.announce = (payload) =>
                callbacks[args.handler]({ event: args.event, id: 1, payload });
            return 1;
          }
          if (command === 'update_install') {
            if (install === 'fail')
              throw { message: 'No update is ready to install.', code: 'not_found' };
            return null;
          }
          throw new Error(`Unexpected desktop command: ${command}`);
        },
      };
    },
    { desktop, status, install },
  );
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.goto(`${base}/tests/update-notice.html${query}`);
  return { page, context };
}
const notice = (page) => page.getByRole('status').filter({ hasText: /is ready|È pronta/ });

try {
  {
    // Nothing ready at load: no notice, until the backend announces a package.
    const { page, context } = await open();
    await expect
      .poll(() =>
        page.evaluate(
          () => window.calls.includes('update_status') && typeof window.announce === 'function',
        ),
      )
      .toBe(true);
    await expect(notice(page)).toHaveCount(0);
    await page.evaluate((info) => window.announce(info), ready);
    await expect(notice(page)).toBeVisible();
    await expect(notice(page)).toContainText('Qamille 1.2.0 is ready');
    await expect(notice(page)).toContainText(
      'Qamille quits as with Quit, then opens again updated.',
    );
    await expect(notice(page)).toContainText('Faster search.');
    // It stays: a toast would be gone after 4.5 s.
    await page.waitForTimeout(5500);
    await expect(notice(page)).toBeVisible();
    await notice(page).getByRole('button', { name: 'Restart and update', exact: true }).click();
    await expect
      .poll(() =>
        page.evaluate(() => window.calls.filter((command) => command === 'update_install').length),
      )
      .toBe(1);
    // After the call (the quit is the guards' to decide) the button is ready again and the notice is still here.
    await expect(
      notice(page).getByRole('button', { name: 'Restart and update', exact: true }),
    ).toBeEnabled();
    await expect(notice(page)).toBeVisible();
    if (shots) await page.screenshot({ path: `${shots}/update-notice-light.png` });
    await context.close();
    console.log(
      'PASS the notice appears when a package is announced, stays, and «Restart and update» calls update_install',
    );
  }
  {
    // A package ready before the page loaded (a reload): asked with update_status. In Italian.
    const { page, context } = await open({ status: ready, query: '?lang=it&theme=dark' });
    await expect(notice(page)).toBeVisible();
    await expect(notice(page)).toContainText('È pronta la versione 1.2.0 di Qamille');
    await expect(notice(page)).toContainText(
      'Qamille si chiude come con Esci, poi si riapre aggiornato.',
    );
    await expect(
      notice(page).getByRole('button', { name: 'Riavvia e aggiorna', exact: true }),
    ).toBeVisible();
    if (shots) await page.screenshot({ path: `${shots}/update-notice-dark-it.png` });
    await context.close();
    console.log('PASS a package ready before the page loaded shows at once, in Italian too');
  }
  {
    // update_install refuses: the error goes to the app's error notice, and the notice stays.
    const { page, context } = await open({
      status: { version: '1.2.0', notes: '' },
      install: 'fail',
    });
    await expect(notice(page)).toBeVisible();
    await expect(notice(page).locator('.update-notice-notes')).toHaveCount(0);
    await notice(page).getByRole('button', { name: 'Restart and update', exact: true }).click();
    await expect
      .poll(() => page.evaluate(() => window.noticeErrors))
      .toEqual(['No update is ready to install.']);
    await expect(notice(page)).toBeVisible();
    await context.close();
    console.log('PASS a refused install reaches the error notice, and the notice stays');
  }
  {
    // The browser preview: no desktop, no question to it (a call would load the preview's commands),
    // no notice.
    const requested = [];
    const { page, context } = await open({ desktop: false });
    page.on('request', (request) => requested.push(new URL(request.url()).pathname));
    await page.reload();
    await page.locator('#fixture').waitFor({ state: 'attached' });
    await page.waitForTimeout(1000);
    await expect(page.locator('.update-notice')).toHaveCount(0);
    assert.ok(
      requested.some((file) => file.endsWith('/UpdateNotice.tsx')),
      'the fixture loaded the notice',
    );
    assert.ok(
      !requested.some((file) => file.endsWith('/lib/preview.ts')),
      'the notice asked the preview nothing',
    );
    await context.close();
    console.log('PASS outside the desktop the notice asks nothing and shows nothing');
  }
  assert.deepEqual(errors, [], 'no page errors');
} finally {
  await browser.close();
}
