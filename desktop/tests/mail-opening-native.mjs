// Verify the compiled desktop reader without opening any real mail account.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '../..');
const data = path.join(root, 'checks/mail-opening', `native-${Date.now()}`);
const port = 28153;
await fs.mkdir(data, { recursive: true });
const child = spawn(path.join(root, 'app/target/release/qamille-desktop.exe'), [], {
  cwd: root, windowsHide: true, stdio: 'pipe',
  env: { ...process.env, QAMILLE_DATA: data, QAMILLE_DEMO: '1',
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
});
let output = '';
child.stdout.on('data', chunk => output += chunk);
child.stderr.on('data', chunk => output += chunk);
let browser, page;
try {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (child.exitCode !== null) throw new Error(`Sample app exited: ${output}`);
    try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) break; } catch {}
    if (attempt === 99) throw new Error(`Sample app did not start: ${output}`);
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  page = browser.contexts()[0].pages()[0];
  await expect(page.locator('.message-body')).toBeVisible();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluate(async () => {
    const workspace = await window.__TAURI_INTERNALS__.invoke('workspace_snapshot');
    if (!workspace.accounts.length || workspace.accounts.some(account => account.kind !== 'Sample')) throw new Error('Sample accounts required');
  });
  const target = page.locator('.mail-row').nth(1);
  const subject = (await target.locator('.mail-subject').innerText()).trim();
  await page.evaluate(async subject => {
    const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args);
    const list = await invoke('list_mail', { query: { folder: 'Inbox' } });
    const item = list.items.find(item => item.subject === subject);
    if (!item) throw new Error('Sample message missing');
    const before = await invoke('read_mail', { id: item.id });
    await invoke('prefetch_mail', { ids: [item.id] });
    const after = await invoke('read_mail', { id: item.id });
    if (after.read !== before.read) throw new Error('Prefetch must not mark messages read');
  }, subject);
  await target.locator('.mail-open').click();
  await expect(target).toHaveClass(/selected/, { timeout: 1000 });
  await expect(page.locator('.message-title')).toHaveText(subject, { timeout: 1000 });
  await expect(page.locator('.message-body')).toBeVisible();
  await expect(page.locator('.reader')).toHaveAttribute('aria-busy', 'false');
  await page.screenshot({ path: path.join(root, 'checks/mail-opening/native-reader.png') });
  assert.deepEqual(errors, []);
  console.log('PASS Compiled Release supports prefetch without marking mail read and opens the selected sample within one second');
} finally {
  if (page && !page.isClosed()) {
    const closed = page.waitForEvent('close', { timeout: 10000 });
    await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' })).catch(() => {});
    await closed.catch(() => {});
  }
  await browser?.close();
  // This child uses the disposable sample workspace created above.
  if (child.exitCode === null) child.kill();
}
