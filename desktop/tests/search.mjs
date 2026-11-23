import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { chromium } from '@playwright/test';

import { pickDate } from './support/dates.mjs';
const source = fs.readFileSync(new URL('../src/lib/search.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const search = await import(
  `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
);
assert.deepEqual(search.queryParts('from:maria subject:"contratto firmato" (to:a OR to:b)'), [
  'from:maria',
  'subject:"contratto firmato"',
  '(to:a OR to:b)',
]);
assert.equal(
  search.withSearchFilter('(from:a OR from:b) around:2026-10-15/6d', 'around', '2026-10-15/14d'),
  '(from:a OR from:b) around:2026-10-15/14d',
);
assert.equal(
  search.withSearchFilter('from:a OR from:b', 'subject', 'contract'),
  '(from:a OR from:b) subject:contract',
);
assert.equal(search.removeSearchChip('(from:a OR from:b) subject:contract', 0), '');
assert.deepEqual(search.aroundRange('around:2026-10-15'), {
  day: '2026-10-15',
  radius: 6,
  start: '2026-10-09',
  end: '2026-10-21',
});
assert.equal(search.offsetISO('2028-02-29', 1), '2028-03-01');
assert.equal(search.canonicalSearchQuery('around:15/10/2026/3d', 'it-IT'), 'around:2026-10-15/3d');
assert.equal(search.canonicalSearchQuery('on:10/03/2026', 'en-US'), 'on:2026-10-03');
assert.equal(search.canonicalSearchQuery('on:10/03/2026', 'it-IT'), 'on:2026-03-10');
assert.equal(search.canonicalSearchDate('2026-02-31'), null);
assert.equal(search.recoverySuggestions('(from:a in:inbox OR from:b)', 'en-US', false).length, 0);
assert.equal(
  search.recoverySuggestions('from:maria between:2026-10-01..2026-10-31', 'it-IT', false)[0].query,
  'from:maria between:2026-09-24..2026-11-07',
);
assert.deepEqual(search.markedParts('A proposal for Q4', ['proposal']), [
  { text: 'A ', match: false },
  { text: 'proposal', match: true },
  { text: ' for Q4', match: false },
]);
console.log('PASS query builders preserve boolean groups, quoted phrases and calendar ranges');
if (process.argv.includes('--utilities-only')) process.exit(0);

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({
  viewport: { width: 1440, height: 1000 },
  locale: 'en-US',
  timezoneId: 'Europe/Rome',
});
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
const screenshots = new URL('../../checks/search-ui/', import.meta.url);
fs.mkdirSync(screenshots, { recursive: true });

async function settled() {
  await page.locator('.search-progress').waitFor({ state: 'hidden' });
}
async function query(text) {
  const input = page.getByRole('textbox', { name: 'Search mail', exact: true });
  await input.fill(text);
  await input.press('Enter');
  await settled();
}

try {
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.getByRole('textbox', { name: 'Search mail', exact: true }).waitFor();
  assert.equal(
    await page.evaluate(() => '__TAURI_INTERNALS__' in window),
    false,
    'Search UI tests must use the isolated browser preview',
  );

  await page.getByRole('button', { name: /^When/ }).click();
  // The date field is Sigrid's: a button that opens a calendar, where the day is chosen.
  await pickDate(page, page.getByLabel('Date', { exact: true }), '2026-10-15');
  assert.match(
    await page.locator('.search-date-preview').innerText(),
    /October 9, 2026.*October 21, 2026/s,
  );
  assert.match(await page.locator('.search-date-preview').innerText(), /13 days included/);
  await page.screenshot({
    path: new URL('around-desktop.png', screenshots).pathname.replace(/^\/([A-Z]:)/, '$1'),
  });
  await page.setViewportSize({ width: 900, height: 900 });
  const panel = await page.locator('.search-filter-panel').boundingBox();
  assert.ok(
    panel && panel.x >= 0 && panel.x + panel.width <= 900,
    'Date filter remains in the viewport at 900px',
  );
  await page.screenshot({
    path: new URL('around-900.png', screenshots).pathname.replace(/^\/([A-Z]:)/, '$1'),
  });
  await page.getByRole('button', { name: 'Apply filter', exact: true }).click();
  assert.equal(
    await page.getByRole('textbox', { name: 'Search mail', exact: true }).inputValue(),
    'around:2026-10-15/6d',
  );
  await settled();
  console.log('PASS visual around-date filter includes ±6 days and fits desktop/narrow viewports');

  await page.getByRole('button', { name: 'I can’t find it', exact: true }).click();
  await page.getByRole('button', { name: 'Try 14 days before and after', exact: true }).click();
  assert.equal(
    await page.getByRole('textbox', { name: 'Search mail', exact: true }).inputValue(),
    'around:2026-10-15/14d',
  );
  await settled();
  await page.getByRole('button', { name: 'Undo change', exact: true }).click();
  assert.equal(
    await page.getByRole('textbox', { name: 'Search mail', exact: true }).inputValue(),
    'around:2026-10-15/6d',
  );
  console.log('PASS recovery widens one filter and undo restores the original query');

  await page.setViewportSize({ width: 1440, height: 1000 });
  await query('Dinner');
  await page.locator('.mail-open', { hasText: 'Dinner on Saturday?' }).waitFor();
  const before = await page.locator('.mail-open').count();
  await query('around:2026-02-31');
  await page.locator('.search-error').waitFor();
  assert.equal(
    await page.locator('.mail-open').count(),
    before,
    'Invalid queries keep the last valid results',
  );
  assert.match(await page.locator('.search-error').innerText(), /Dinner/);
  await page.getByRole('button', { name: 'Search as text', exact: true }).click();
  assert.equal(
    await page.getByRole('textbox', { name: 'Search mail', exact: true }).inputValue(),
    '"around:2026-02-31"',
  );
  await settled();
  console.log(
    'PASS invalid date stays visible, preserves results, and offers literal-text recovery',
  );

  await query('');
  await page.getByRole('button', { name: /^From/ }).click();
  await page.getByPlaceholder('Name or email address').fill('maria@client-example.com');
  await page.screenshot({
    path: new URL('person-desktop.png', screenshots).pathname.replace(/^\/([A-Z]:)/, '$1'),
  });
  await page.getByRole('button', { name: 'Apply filter', exact: true }).click();
  await settled();
  await page
    .locator('.mail-open', { hasText: 'Q4 proposal: can you review by Thursday?' })
    .waitFor();
  assert.equal(await page.locator('.mail-open').count(), 1);
  await page.getByRole('button', { name: /^When/ }).click();
  await page.getByLabel('Date', { exact: true }).press('Escape');
  assert.equal(
    await page
      .getByRole('textbox', { name: 'Search mail', exact: true })
      .evaluate((input) => input === document.activeElement),
    true,
  );
  console.log('PASS exact sender filtering and Escape restores search focus');

  await query('(from:maria@client-example.com OR from:alex.home@example.org)');
  assert.equal(await page.locator('.search-chip').count(), 1);
  await page.locator('.search-chip-remove').click();
  assert.equal(
    await page.getByRole('textbox', { name: 'Search mail', exact: true }).inputValue(),
    '',
  );
  console.log('PASS boolean query chips preserve grouping and clear safely');

  await page.getByRole('button', { name: 'Alex Personal workspace' }).click();
  await page.getByRole('combobox', { name: 'Regional format', exact: true }).click();
  await page.getByRole('option', { name: 'it-IT', exact: true }).click();
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await page.locator('.sidebar-body nav').nth(1).locator('button').first().click();
  await page.getByRole('textbox', { name: 'Cerca email', exact: true }).waitFor();
  await page.getByRole('button', { name: /^Quando/ }).click();
  // The date field (DateField) reads what is typed on Enter or when the focus leaves it.
  await page.getByLabel('Data', { exact: true }).fill('2026-10-15');
  await page.getByLabel('Data', { exact: true }).press('Enter');
  assert.match(
    await page.locator('.search-date-preview').innerText(),
    /9 ottobre 2026.*21 ottobre 2026/s,
  );
  await page.screenshot({
    path: new URL('around-italian.png', screenshots).pathname.replace(/^\/([A-Z]:)/, '$1'),
  });
  console.log('PASS Italian search labels and readable dates');
  assert.deepEqual(errors, [], `Uncaught browser errors: ${errors.join('; ')}`);
} catch (error) {
  await page
    .screenshot({ path: new URL('failure.png', screenshots).pathname.replace(/^\/([A-Z]:)/, '$1') })
    .catch(() => {});
  console.error('Browser errors:', errors);
  throw error;
} finally {
  await browser.close();
}
