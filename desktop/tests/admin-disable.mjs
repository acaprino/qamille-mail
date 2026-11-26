// Exercise the real admin page with synthetic accounts on a touch viewport and with the keyboard.
import { readFile } from 'node:fs/promises';
import { chromium, expect } from '@playwright/test';

const assets = new Map(await Promise.all(['index.html', 'app.js', 'app.css'].map(async name => [
  name, await readFile(new URL('../../server/src/http/admin/' + name, import.meta.url), 'utf8'),
])));
const browser = await chromium.launch({ channel: 'msedge', headless: true });
try {
  for (const touch of [true, false]) {
    const page = await browser.newPage({ hasTouch: touch, viewport: { width: touch ? 390 : 1200, height: 844 } });
    let disables = 0;
    const user = { id: 'one', username: 'Ada', active_devices: 2, disabled_at: null };
    await page.route('https://admin.example/**', async route => {
      const { pathname } = new URL(route.request().url());
      if (pathname === '/admin/api/users')
        return route.fulfill({ json: { users: [user], default_max_devices: 4 } });
      if (pathname === '/admin/api/users/one/devices') return route.fulfill({ json: { devices: [] } });
      if (pathname === '/admin/api/users/one/disable' && route.request().method() === 'POST') {
        disables++;
        user.disabled_at = '2026-01-01T00:00:00Z';
        user.active_devices = 0;
        return route.fulfill({ status: 204 });
      }
      const name = pathname === '/admin' ? 'index.html' : pathname.split('/').pop();
      if (!assets.has(name)) return route.fulfill({ status: 404, json: {} });
      const types = { 'index.html': 'text/html', 'app.js': 'application/javascript', 'app.css': 'text/css' };
      return route.fulfill({ contentType: types[name], body: assets.get(name) });
    });
    await page.clock.install();
    await page.goto('https://admin.example/admin');
    await page.locator('#key-input').fill('synthetic-admin');
    await page.getByRole('button', { name: 'Entra' }).click();
    await page.getByRole('button', { name: 'Gestisci' }).click();
    const warning = page.getByText(/disconnetti tutti i suoi dispositivi/);
    await expect(warning).toBeVisible();
    const choose = button => touch ? button.tap() : button.press('Enter');
    await choose(page.getByRole('button', { name: 'Disattiva', exact: true }));
    const confirm = page.getByRole('button', { name: 'Sì, disattiva' });
    await expect(confirm).toBeVisible();
    expect(disables).toBe(0);
    await page.clock.fastForward(600);
    await choose(confirm);
    await expect(page.getByRole('button', { name: 'Riattiva' })).toBeVisible();
    expect(disables).toBe(1);
    await page.close();
  }
} finally {
  await browser.close();
}
