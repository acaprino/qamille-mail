// «Cerca sul server» in pages: the first 50 results, then «Altri risultati dal server» for the
// rest, with the coverage notice saying how much of the mailbox the search covered.
// Run against the browser preview: QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
import { chromium } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
await page.addInitScript(() => {
  window.__previewRemote = { total: 70 };
});

const input = () => page.getByRole('textbox', { name: 'Search mail', exact: true });
const notice = () => page.locator('.search-coverage');
const settled = () => page.locator('.search-progress').waitFor({ state: 'hidden' });

try {
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await input().waitFor();
  await input().fill('fattura');
  await input().press('Enter');
  await settled();
  assert.match(await notice().innerText(), /Searching \d+ of \d+ messages in this account\./);
  await page.getByRole('button', { name: 'Search server', exact: true }).click();
  const more = page.getByRole('button', { name: 'More results from the server', exact: true });
  await more.waitFor();
  assert.match(await notice().innerText(), /Results from the server: 50 of 70\./);
  assert.equal(await page.getByRole('button', { name: 'Search server', exact: true }).count(), 0);
  assert.ok((await page.getByText(/fattura · server \d+/).count()) > 0, 'server results are rows of the list');
  console.log('PASS the first page of server results is listed with its coverage');

  await more.click();
  await page.waitForFunction(() => /70 of 70/.test(document.querySelector('.search-coverage')?.textContent ?? ''));
  await settled();
  assert.equal(await more.count(), 0, 'no more results once the server has given them all');
  assert.equal(await page.getByRole('button', { name: 'Search server', exact: true }).count(), 0);
  console.log('PASS «More results from the server» brings the rest and then disappears');

  await input().fill('bolletta');
  await input().press('Enter');
  await settled();
  assert.equal(await more.count(), 0, 'another search starts from its own first page');
  await page.getByRole('button', { name: 'Search server', exact: true }).waitFor();
  console.log('PASS a new search does not inherit the previous cursor');
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
