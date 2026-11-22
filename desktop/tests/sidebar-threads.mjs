import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const fixture = JSON.parse(
  await fs.readFile(new URL('../src/lib/sample.json', import.meta.url), 'utf8'),
);
const anchor = fixture.emails.find(
  (email) => email.folder === 'Inbox' && !email.done && !email.snoozed_until,
);
anchor.message_id = 'sidebar-root@test';
anchor.thread_id = '';
anchor.pinned = true;
const reply = {
  ...anchor,
  id: 99001,
  message_id: 'sidebar-reply@test',
  thread_id: '<sidebar-root@test>',
  date: anchor.date + 60,
  body: 'A reply in the same conversation.',
  read: true,
  pinned: false,
};
fixture.emails.push(reply, {
  ...reply,
  id: 99002,
  account_id: fixture.accounts.find((a) => a.id !== anchor.account_id).id,
  body: 'Separate account conversation.',
});
fixture.next_id = 100000;
fixture.undo_actions = [];
if (process.argv.includes('--write-fixture')) {
  const directory = new URL('../../checks/sidebar-native/', import.meta.url);
  await fs.mkdir(directory, { recursive: true });
  await fs.writeFile(new URL('store.json', directory), JSON.stringify(fixture));
  console.log('Prepared isolated sample workspace for desktop verification');
  process.exit(0);
}
const native = process.env.QAMILLE_CDP;
const browser = native
  ? await chromium.connectOverCDP(native)
  : await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const page = native
  ? browser.contexts()[0].pages()[0]
  : await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 1000 } });
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
if (!native)
  await page.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({
      contentType: 'application/javascript',
      body: `export default ${JSON.stringify(fixture)};`,
    }),
  );
try {
  if (!native) await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  else {
    const accounts = await page.evaluate(
      async () => (await window.__TAURI_INTERNALS__.invoke('workspace_snapshot')).accounts,
    );
    assert.ok(
      accounts.length > 0 && accounts.every((account) => account.kind === 'Sample'),
      'Desktop verification requires only sample accounts',
    );
    await page.evaluate(() => {
      localStorage.removeItem('qamille:mail-display');
      localStorage.removeItem('qamille:sidebar-collapsed');
    });
    await page.reload();
  }
  await page.getByRole('button', { name: 'Threads', exact: true }).click({ timeout: 5000 });
  await expect(page.getByRole('button', { name: 'Threads', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  const rows = page.locator('.mail-row');
  // The conversation row says how many messages it holds on its arrow (ADR-0010).
  const threadRow = rows.filter({ has: page.locator('.thread-toggle', { hasText: '2' }) });
  await expect(threadRow).toHaveCount(1);
  const badge = await threadRow.locator('.thread-toggle').boundingBox();
  // The arrow stays inside its row and never over the subject text.
  const rowBox = await threadRow.boundingBox();
  const subject = await threadRow.locator('.mail-subject-text').boundingBox();
  assert.ok(
    badge.x >= rowBox.x && badge.x + badge.width <= rowBox.x + rowBox.width + 1,
    'thread count remains visible inside its row',
  );
  // Compatto: the arrow is on the first line of the row (by the time), the subject on the second.
  assert.ok(
    subject.x + subject.width <= badge.x + 1 || subject.y >= badge.y + badge.height - 1 || badge.y >= subject.y + subject.height - 1,
    'and never sits on the subject text',
  );
  // The conversation is in the list (ADR-0010): the reader no longer repeats it, and the other
  // message is a row under the main one that reads in the pane.
  await threadRow.locator('.mail-open').click();
  await expect(page.getByRole('region', { name: 'Conversation messages' })).toHaveCount(0);
  await threadRow.locator('.thread-toggle').click();
  await expect(page.locator('.reply-row')).toHaveCount(1);
  await page.locator('.reply-row .reply-open').click();
  await expect(page.locator('.message-body')).toHaveText(anchor.body);
  await expect(page.locator('.reply-row.selected')).toHaveCount(1);
  await threadRow.locator('.mail-open').click();
  await expect(threadRow).toHaveClass(/selected/);
  await threadRow.getByRole('button', { name: 'Unpin thread', exact: true }).click();
  await expect(threadRow.getByRole('button', { name: 'Pin thread', exact: true })).toBeVisible();
  await page.screenshot({
    path: native ? '../checks/sidebar-threads-native.png' : '../checks/sidebar-threads.png',
  });
  await threadRow.getByRole('checkbox').check();
  await page.getByRole('button', { name: 'Print selected', exact: true }).click();
  await expect(page.locator('.print-message')).toHaveCount(2);
  await page.getByRole('button', { name: 'Close print preview' }).click();
  await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
  await expect(page.locator('.sidebar')).toHaveClass(/collapsed/);
  assert.ok((await page.locator('.sidebar').boundingBox()).width <= 76);
  await page
    .getByRole('button', { name: `Switch to ${fixture.accounts[0].name}`, exact: true })
    .click();
  await expect(page.locator('.workspace-name')).toContainText(fixture.accounts[0].name);
  await page.reload();
  await expect(page.locator('.sidebar')).toHaveClass(/collapsed/);
  await expect(page.getByRole('button', { name: 'Threads', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await page.getByRole('button', { name: 'Emails', exact: true }).click();
  await expect(page.locator('.thread-toggle')).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Conversation messages' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Expand sidebar', exact: true }).click();
  await expect(page.locator('.sidebar')).not.toHaveClass(/collapsed/);
  if (!native) await page.setViewportSize({ width: 850, height: 650 });
  await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
  assert.ok((await page.locator('.sidebar').boundingBox()).width <= 76);
  const folders = page.getByRole('navigation', { name: 'Mail folders' });
  await folders.getByRole('button', { name: 'Sent', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Sent', exact: true })).toBeVisible();
  // Account buttons filter mail, so pages without mail must hand over to the inbox.
  const leaveDialog = page.getByRole('dialog', { name: 'Save your changes?' });
  const other = fixture.accounts[1];
  await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await page.getByRole('button', { name: `Switch to ${other.name}`, exact: true }).click();
  // Synthetic sample settings may include optional legacy keys; discard any form-only defaults.
  if (await leaveDialog.isVisible()) await leaveDialog.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible();
  await expect(page.locator('.workspace-name')).toContainText(other.name);
  await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('textbox', { name: 'Your name', exact: true }).fill('Unsaved account switch');
  await page.getByRole('button', { name: 'All accounts', exact: true }).click();
  await expect(leaveDialog).toBeVisible();
  await leaveDialog.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `Switch to ${other.name}`, exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await page.getByRole('button', { name: 'All accounts', exact: true }).click();
  await leaveDialog.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible();
  await expect(page.locator('.workspace-name')).toContainText('All accounts');
  assert.deepEqual(errors, []);
  await page.screenshot({
    path: native ? '../checks/sidebar-collapsed-native.png' : '../checks/sidebar-collapsed.png',
  });
  console.log(
    'PASS conversation membership, reader navigation, selection, account icons, preferences, narrow layout',
  );
} finally {
  if (native)
    // The close button closes Qamille in this test window, instead of putting it in the tray.
    await page
      .evaluate(async () => {
        await window.__TAURI_INTERNALS__.invoke('close_to_tray_set', { on: false });
        await window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' });
      })
      .catch(() => {});
  await browser.close();
}
