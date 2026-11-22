// Mount the real settings panel with synthetic IPC; never changes assistant or account files.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:5187';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({ locale: 'en-US' });
const page = await context.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(() => {
  window.isTauri = true;
  window.settingsCalls = [];
  window.settingsNotices = [];
  let failLoad = window.location.search.includes('failure');
  const limited = { access: 'ReadOnly', account_ids: [], calendar_ids: [], ai_execute: false, permanent_delete: false };
  const broad = { ...limited, access: 'Send', account_ids: null, calendar_ids: null };
  let config = { profiles: { default: window.location.search.includes('switch') ? broad : limited, limited }, worker_autostart: false };
  window.__TAURI_INTERNALS__ = {
    async invoke(command, args = {}) {
      window.settingsCalls.push({ command, args: structuredClone(args) });
      if (command === 'mcp_configuration') {
        if (failLoad) { failLoad = false; throw new Error('Synthetic permissions unavailable'); }
        return { config: structuredClone(config), worker_running: false, client_config: {} };
      }
      if (command === 'assistant_installation_status') return { targets: [
        { id: 'codex', name: 'Codex', mode: 'direct', detected: true, installed: true, prepared: true, requires_import: false, supports_mcp: true, supports_skill: true },
        { id: 'chatgpt', name: 'ChatGPT Work', mode: 'package', detected: true, installed: false, prepared: true, requires_import: true, supports_mcp: true, supports_skill: true },
      ], executable_path: 'C:\\Synthetic\\Qamille.exe' };
      if (command === 'save_mcp_configuration') { config = structuredClone(args.config); return null; }
      throw new Error(`Unexpected command: ${command}`);
    },
  };
});
await page.route('**/__mcp-settings-test*', route => route.fulfill({
  contentType: 'text/html',
  body: '<!doctype html><html><head><link rel="stylesheet" href="/src/styles.css"></head><body><main id="fixture"></main><script type="module">const { mount } = await import("/tests/support/react-mount.js"); const { default: Component } = await import("/src/components/McpSettings.tsx"); mount(Component, { target: document.querySelector("#fixture"), props: { workspace: {settings:{mcp_access:"Send"},accounts:[{id:1,email:"synthetic@example.test"}]}, notify: (message,error)=>window.settingsNotices.push({message,error}) } });</script></body></html>',
}));

// The Ark Select and Switch hide their native input: read the state by role, click the visible track.
const permission = () => page.getByRole('combobox', { name: 'Profile permission', exact: true });
const option = name => page.getByRole('switch', { name, exact: true });
const flip = name => page.locator('[data-scope="switch"][data-part="root"]', { hasText: name }).locator('[data-part="control"]').click();
const save = () => page.getByRole('button', { name: 'Save assistant permissions', exact: true });
const savedProfile = name => page.evaluate(profileName => window.settingsCalls.filter(call => call.command === 'save_mcp_configuration').at(-1)?.args.config.profiles[profileName], name);
try {
  await page.goto(`${base}/__mcp-settings-test?case=default`);
  await expect(save()).toBeEnabled();
  await expect(permission()).toContainText('Read only');
  await expect(option('All accounts')).not.toBeChecked();
  await expect(option('All calendars')).not.toBeChecked();
  await save().click();
  assert.deepEqual(await savedProfile('default'), { access: 'ReadOnly', account_ids: [], calendar_ids: [], ai_execute: false, permanent_delete: false });
  console.log('PASS saved default profile loads and preserves deny-all account/calendar permissions');

  await page.goto(`${base}/__mcp-settings-test?case=switch`);
  await expect(save()).toBeEnabled();
  await page.getByRole('textbox', { name: 'Assistant profile', exact: true }).fill('limited');
  await expect(permission()).toContainText('Read only');
  await flip('Allow AI processing of messages and meetings');
  await save().click();
  assert.deepEqual(await savedProfile('limited'), { access: 'ReadOnly', account_ids: [], calendar_ids: [], ai_execute: true, permanent_delete: false });
  console.log('PASS switching from Send/all to a limited profile cannot retain the previous grants');

  const allowedCalendars = page.getByRole('textbox', { name: 'Allowed calendars (one per line; empty = none)', exact: true });
  await allowedCalendars.fill('local\nprimary');
  await save().click();
  assert.deepEqual((await savedProfile('limited')).calendar_ids, ['local', 'primary']);
  await flip('All calendars');
  await expect(option('All calendars')).toBeChecked();
  await save().click();
  assert.equal((await savedProfile('limited')).calendar_ids, null);
  await flip('All calendars');
  await expect(option('All calendars')).not.toBeChecked();
  await allowedCalendars.fill('');
  await save().click();
  assert.deepEqual((await savedProfile('limited')).calendar_ids, []);
  console.log('PASS calendar scopes distinguish selected calendars, all calendars, and no calendars');

  await expect(page.getByRole('region', { name: 'Codex', exact: true }).locator('.installer-status')).toHaveText('Configured');
  await expect(page.getByRole('region', { name: 'ChatGPT Work', exact: true }).locator('.installer-status')).toHaveText('Ready to import');
  console.log('PASS direct installation and packages awaiting import have distinct status labels');

  await page.goto(`${base}/__mcp-settings-test?case=failure`);
  await expect(page.getByRole('alert')).toContainText('Synthetic permissions unavailable');
  await expect(save()).toHaveCount(0);
  assert.equal(await page.evaluate(() => window.settingsCalls.some(call => call.command === 'save_mcp_configuration')), false);
  await page.getByRole('button', { name: 'Reload permissions', exact: true }).click();
  await expect(save()).toBeEnabled();
  await save().click();
  assert.deepEqual((await savedProfile('default')).calendar_ids, []);
  assert.deepEqual(errors, []);
  console.log('PASS failed configuration load blocks saving and can be retried without widening grants');
} finally { await browser.close(); }
