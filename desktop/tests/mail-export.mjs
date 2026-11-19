import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const fixture = JSON.parse(
  await fs.readFile(new URL('../src/lib/sample.json', import.meta.url), 'utf8'),
);
const anchor = fixture.emails.find((e) => e.attachments.length && e.folder === 'Inbox');
const reply = {
  ...anchor,
  id: 98001,
  folder: 'Sent',
  subject: `Re: ${anchor.subject}`,
  body: 'Reply outside the inbox. Caffè ✓\n<script>window.compromised = true</script>',
  attachments: [],
  date: anchor.date + 60,
  read: true,
  message_id: 'reply@example.com',
};
const foreign = {
  ...reply,
  id: 98002,
  account_id: anchor.account_id === 1 ? 2 : 1,
  body: 'OTHER ACCOUNT MUST NOT PRINT',
};
fixture.emails.push(reply, foreign);
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const page = await browser.newPage({
  locale: 'en-US',
  viewport: { width: 1440, height: 900 },
  acceptDownloads: true,
});
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));
await page.route(/\/src\/lib\/sample\.json/, (route) =>
  route.fulfill({
    contentType: 'application/javascript',
    body: `export default ${JSON.stringify(fixture)};`,
  }),
);
await page.addInitScript(() => {
  window.printCalls = 0;
  window.print = () => {
    window.printCalls += 1;
  };
});
try {
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.locator('.mail-open', { hasText: `${anchor.subject}` }).click();
  await page.getByRole('button', { name: 'More message actions' }).click();
  await page.getByRole('button', { name: 'Print email', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Email print preview' })).toBeVisible();
  await expect(page.locator('.print-message')).toHaveCount(1);
  await expect(page.locator('.print-message')).toContainText(anchor.body);
  await page.getByRole('button', { name: 'Print / Save PDF' }).click();
  assert.equal(await page.evaluate(() => window.printCalls), 1);
  await page.getByRole('button', { name: 'Close print preview' }).click();
  console.log('PASS single message preview and print dialog');

  await page.getByRole('button', { name: 'More message actions' }).click();
  await page.getByRole('button', { name: 'Print thread', exact: true }).click();
  await expect(page.locator('.print-message')).toHaveCount(2);
  await expect(page.locator('.print-message').nth(1)).toContainText(reply.body);
  assert.equal(await page.evaluate(() => window.compromised), undefined);
  await page.emulateMedia({ media: 'print' });
  assert.equal(
    await page.locator('.print-controls').evaluate((e) => getComputedStyle(e).display),
    'none',
  );
  assert.equal(await page.locator('.sidebar').evaluate((e) => e.getBoundingClientRect().height), 0);
  assert.ok(
    (await page
      .locator('.print-message')
      .first()
      .evaluate((e) => e.getBoundingClientRect().height)) > 100,
  );
  await page.pdf({
    path: '../checks/email-thread-print.pdf',
    format: 'A4',
    preferCSSPageSize: true,
  });
  await page.emulateMedia({ media: 'screen' });
  await page.screenshot({ path: '../checks/email-print-preview.png' });
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Email print preview' })).toHaveCount(0);
  console.log('PASS complete thread, sent mail, account isolation, safe text and print layout');

  const emlDownload = page.waitForEvent('download');
  await page.getByRole('button', { name: 'More message actions' }).click();
  await page.getByRole('button', { name: 'Download .eml', exact: true }).click();
  const eml = await emlDownload;
  assert.ok(eml.suggestedFilename().endsWith('.eml'));
  await eml.saveAs('../checks/email-export.eml');
  const emlText = await fs.readFile(await eml.path(), 'utf8');
  assert.match(emlText, /Content-Type: multipart\/mixed;/);
  assert.match(emlText, /Content-Disposition: attachment;/);
  console.log('PASS EML download with attachment MIME');

  await expect(page.getByRole('button', { name: 'Download .mbox' })).toHaveCount(0);
  const checkboxes = page.locator('.mail-checkbox');
  await checkboxes.nth(0).check();
  await checkboxes.nth(1).check();
  const mboxDownload = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Download .mbox' }).click();
  const mbox = await mboxDownload;
  assert.equal(mbox.suggestedFilename(), 'qamille-2-messages.mbox');
  await mbox.saveAs('../checks/email-export.mbox');
  const mboxText = await fs.readFile(await mbox.path(), 'utf8');
  assert.equal(mboxText.match(/^From /gm).length, 2);
  await page.getByRole('button', { name: 'Print selected' }).click();
  await expect(page.locator('.print-message')).toHaveCount(2);
  await page.getByRole('button', { name: 'Close print preview' }).click();
  // The bar's checkbox is the Checkbox primitive: its input is hidden, the drawn box is what takes the click.
  await page.getByRole('checkbox', { name: 'Select loaded messages' }).check({ force: true });
  assert.equal(await checkboxes.count(), await page.locator('.mail-checkbox:checked').count());
  await page.getByRole('tab', { name: 'Unread', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Download .mbox' })).toHaveCount(0);
  await checkboxes.first().check();
  await page.getByRole('textbox', { name: 'Search mail' }).fill('Dinner');
  await expect(page.getByRole('button', { name: 'Download .mbox' })).toHaveCount(0);
  console.log('PASS MBOX download, batch print, select loaded, selection reset on filter/search');

  await page.getByRole('textbox', { name: 'Search mail' }).fill('');
  await page.getByRole('tab', { name: 'All', exact: true }).click();
  await expect(checkboxes.first()).toBeVisible();
  await checkboxes.first().check();
  await page.getByRole('button', { name: /^Archive\s*\d*$/ }).click();
  await expect(page.getByRole('button', { name: 'Download .mbox' })).toHaveCount(0);
  await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
  await page.locator('.mail-open', { hasText: `${anchor.subject}` }).click();
  await expect(page.locator('.message-title')).toHaveText(anchor.subject);
  await page.screenshot({ path: '../checks/email-export-inbox.png' });
  assert.deepEqual(errors, []);
  console.log('PASS folder selection reset and zero browser errors');
} finally {
  await browser.close();
}
