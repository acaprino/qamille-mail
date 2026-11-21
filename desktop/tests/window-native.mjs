// Exercise the real window controls against an isolated sample workspace.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '../..');
const data = path.join(root, 'checks/window-controls', `native-${Date.now()}`);
await fs.mkdir(data, { recursive: true });
// The X closes Qamille here, as before the tray (src-tauri/src/background.rs).
await fs.writeFile(path.join(data, 'background.json'), JSON.stringify({ close_to_tray: false, tray_notice_shown: true }));
const executable = path.join(root, 'app/target/debug/qamille-desktop.exe');
const port = 40000 + Math.floor(Math.random() * 10000);
const options = {
  cwd: root, windowsHide: true, stdio: 'pipe',
  env: { ...process.env, QAMILLE_DATA: data, QAMILLE_DEMO: '1',
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    WEBVIEW2_USER_DATA_FOLDER: path.join(data, 'webview') },
};
const child = spawn(executable, [], options);
let output = '';
child.stdout.on('data', bytes => { output += bytes; });
child.stderr.on('data', bytes => { output += bytes; });
let browser, page;
const errors = [];
try {
  for (let attempt = 0; attempt < 80; attempt++) {
    if (child.exitCode !== null) throw new Error(`Test app exited; close other app instances first. ${output}`);
    try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) break; } catch { /* Starting. */ }
    if (attempt === 79) throw new Error(`Test window unavailable: ${output}`);
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  page = browser.contexts()[0].pages()[0];
  page.setDefaultTimeout(15000);
  page.on('pageerror', error => errors.push(error.message));
  const invoke = (command, args = {}) => page.evaluate(({ command, args }) =>
    window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  const windowCall = command => invoke(`plugin:window|${command}`, { label: 'main' });
  await page.locator('.compose-main').waitFor();
  const workspace = await invoke('workspace_snapshot');
  assert.ok(workspace.accounts.length && workspace.accounts.every(account => account.kind === 'Sample'));
  await page.evaluate(() => localStorage.setItem('qamille:ui-language', 'en'));
  await page.reload();
  await page.locator('.compose-main').waitFor();
  const controls = page.getByRole('group', { name: 'Window controls' });
  await expect(controls.getByRole('button')).toHaveCount(3);
  assert.equal(await windowCall('is_decorated'), false);
  assert.equal(await windowCall('is_resizable'), true);
  await page.screenshot({ path: path.join(data, 'borderless-light.png') });
  console.log('PASS native window has no decorations and remains resizable');

  await controls.getByRole('button', { name: 'Maximize window', exact: true }).click();
  await expect.poll(() => windowCall('is_maximized')).toBe(true);
  await expect(controls.getByRole('button', { name: 'Restore window', exact: true })).toBeVisible();
  await controls.getByRole('button', { name: 'Restore window', exact: true }).click();
  await expect.poll(() => windowCall('is_maximized')).toBe(false);
  // Tauri's native drag-region handler also owns double-click maximize/restore.
  await page.locator('.topbar').dispatchEvent('mousedown', { button: 0, buttons: 1, detail: 2 });
  await expect.poll(() => windowCall('is_maximized')).toBe(true);
  await controls.getByRole('button', { name: 'Restore window', exact: true }).click();
  await expect.poll(() => windowCall('is_maximized')).toBe(false);
  console.log('PASS maximize, restore and header double-click');

  await controls.getByRole('button', { name: 'Minimize window', exact: true }).click();
  await expect.poll(() => windowCall('is_minimized')).toBe(true);
  // The existing single-instance handler restores the test window.
  const second = spawn(executable, [], options);
  await new Promise((resolve, reject) => { second.on('exit', resolve); second.on('error', reject); });
  await expect.poll(() => windowCall('is_minimized')).toBe(false);
  await page.getByRole('button', { name: 'Toggle dark appearance' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.screenshot({ path: path.join(data, 'borderless-dark.png') });
  console.log('PASS minimize, restore on relaunch and dark appearance');

  await page.getByTitle('Settings', { exact: true }).click();
  await page.getByRole('textbox', { name: 'Your name', exact: true }).fill('Unsaved window test');
  await controls.getByRole('button', { name: 'Close window', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Save your changes?' })).toBeVisible();
  await page.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(controls).toBeVisible();
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  console.log('PASS close respects unsaved settings and can be cancelled');

  await page.locator('.compose-main').click();
  const panel = page.getByRole('tabpanel').filter({ has: page.getByRole('textbox', { name: 'Message body' }) });
  await panel.getByRole('textbox', { name: 'Subject', exact: true }).fill('Borderless close preserves draft');
  await panel.getByRole('textbox', { name: 'Message body' }).fill('Saved by the custom window close button.');
  const closed = page.waitForEvent('close');
  await controls.getByRole('button', { name: 'Close window', exact: true }).click();
  await closed;
  const store = JSON.parse(await fs.readFile(path.join(data, 'store.json'), 'utf8'));
  assert.ok(store.drafts.some(draft => draft.subject === 'Borderless close preserves draft'
    && draft.body.includes('Saved by the custom window close button.')));
  assert.deepEqual(errors, []);
  console.log('PASS close saves the open draft before exiting');
  console.log(`Artifacts: ${data}`);
} catch (error) {
  await page?.screenshot({ path: path.join(data, 'failure.png') }).catch(() => {});
  throw error;
} finally {
  await browser?.close().catch(() => {});
  if (child.exitCode === null) child.kill();
}
