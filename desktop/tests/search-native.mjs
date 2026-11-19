// Run only against a disposable desktop instance with Sample accounts.
import assert from 'node:assert/strict';
import { chromium } from '@playwright/test';

const browser = await chromium.connectOverCDP(process.env.QAMILLE_CDP || 'http://127.0.0.1:9226');
const page = browser.contexts()[0].pages()[0];
const invoke = (command, args = {}) => page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
const day = (timestamp, zone) => {
  const parts = new Intl.DateTimeFormat('en-CA', { timeZone: zone, year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(new Date(timestamp * 1000));
  const get = (type) => parts.find((p) => p.type === type).value;
  return `${get('year')}-${get('month')}-${get('day')}`;
};
try {
  const workspace = await invoke('workspace_snapshot');
  assert.ok(workspace.accounts.length && workspace.accounts.every((account) => account.kind === 'Sample'), 'Native search tests require Sample accounts only');
  const query = (search, offset = 0, limit = 100) => invoke('list_mail', { query: { search, offset, limit, threads: false } });
  const all = await query('in:anywhere');
  assert.ok(all.items.length);
  assert.equal(all.search.timezone, workspace.regional.time_zone);
  const first = all.items.find((email) => !email.date_missing && email.from_addr);
  const address = first.from_addr;
  const exact = await query(`from:${address} in:anywhere`);
  assert.ok(exact.items.length && exact.items.every((email) => email.from_addr.toLowerCase() === address.toLowerCase()));
  const center = day(first.date, all.search.timezone);
  const on = await query(`around:${center}/0d from:${address} in:anywhere`);
  assert.ok(on.items.length && on.items.every((email) => day(email.date, all.search.timezone) === center));
  const firstPage = await query('in:anywhere', 0, 2);
  const secondPage = await query('in:anywhere', 2, 2);
  assert.equal(firstPage.next_offset, 2);
  assert.ok(secondPage.items.every((email) => !firstPage.items.some((prior) => prior.id === email.id)));
  let invalid = false;
  try { await query('around:2026-02-31'); } catch { invalid = true; }
  assert.ok(invalid, 'Native command must reject invalid dates');
  console.log('PASS native Rust commands: exact sender, original date, configured zone, pagination and validation');

  const input = page.locator('.global-search input');
  await input.fill(`from:${address}`);
  await input.press('Enter');
  await page.locator('.search-progress').waitFor({ state: 'hidden' });
  await page.waitForFunction(() => document.querySelectorAll('.mail-open').length > 0);
  const count = await page.locator('.mail-open').count();
  await input.fill('around:2026-02-31');
  await input.press('Enter');
  await page.locator('.search-error').waitFor();
  assert.equal(await page.locator('.mail-open').count(), count);
  console.log('PASS native UI: shared backend rejects invalid date and preserves prior results');
} finally {
  await browser.close();
}
