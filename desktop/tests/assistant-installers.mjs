// Isolated component checks. All desktop commands are fake: no client files are changed.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

// The Ark Switch hides its native input: read the state by role, click the visible track.
const flip = name => page.locator('[data-scope="switch"][data-part="root"]', { hasText: name }).locator('[data-part="control"]').click();
const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:5187';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({ locale: 'en-US' });
const page = await context.newPage();
const errors = [];
const diagnostics = [];
page.on('pageerror', error => { errors.push(error.message); diagnostics.push(`PAGE ERROR ${error.message}`); });
page.on('console', message => { if (message.type() === 'error') diagnostics.push(`BROWSER ERROR ${message.text()}`); });
page.on('requestfailed', request => diagnostics.push(`REQUEST FAILED ${request.url()} ${request.failure()?.errorText}`));
page.on('response', response => { if (response.status() >= 400) diagnostics.push(`HTTP ERROR ${response.status()} ${response.url()}`); });
await page.addInitScript(() => {
  window.isTauri = true;
  window.installationCalls = [];
  window.installationNotices = [];
  const targets = [
    { id: 'codex', name: 'Codex', mode: 'direct', detected: true, installed: false, supports_mcp: true, supports_skill: true },
    { id: 'claude_code', name: 'Claude Code', mode: 'direct', detected: true, installed: false, supports_mcp: true, supports_skill: true },
    { id: 'claude_desktop', name: 'Claude Desktop', mode: 'direct', detected: false, installed: false, supports_mcp: true, supports_skill: false },
    { id: 'chatgpt', name: 'ChatGPT Work', mode: 'package', detected: true, installed: false, supports_mcp: true, supports_skill: true },
    { id: 'claude_cowork', name: 'Claude Cowork', mode: 'package', detected: true, installed: false, supports_mcp: true, supports_skill: true },
  ];
  window.__TAURI_INTERNALS__ = {
    async invoke(command, args = {}) {
      window.installationCalls.push({ command, args: structuredClone(args) });
      if (command === 'assistant_installation_status') return { targets, executable_path: 'D:\\Synthetic\\Qamille.exe' };
      if (command === 'assistant_installation_preview') return {
        target: args.target, token: `preview-${args.target}-${args.profile}`,
        files: [{ path: `C:\\Synthetic\\${args.target}\\config`, action: 'update' }],
        notes: ['Synthetic preview: existing settings are preserved.'],
        requires_import: targets.find(target => target.id === args.target).mode === 'package',
      };
      if (command === 'assistant_installation_apply') {
        const target = targets.find(target => target.id === args.target);
        target.prepared = target.mode === 'package';
        target.installed = target.mode !== 'package';
        return {
          target: args.target, changed_paths: [`C:\\Synthetic\\${args.target}\\config`],
          backup_paths: [`C:\\Synthetic\\${args.target}\\config.backup`], notes: ['Start a new assistant session.'],
          ...(args.target === 'claude_cowork' ? { package_path: 'C:\\Synthetic\\qamille.zip' } : {}),
        };
      }
      if (command === 'assistant_installation_open') return null;
      throw new Error(`Unexpected command: ${command}`);
    },
  };
});
await page.route('**/__assistant-installers-test', route => route.fulfill({
  contentType: 'text/html',
  body: '<!doctype html><html><head><link rel="stylesheet" href="/src/styles.css"></head><body><main id="fixture" style="max-width:720px;margin:auto"></main><script type="module">const { mount } = await import("/tests/support/react-mount.js"); const { default: Component } = await import("/src/components/AssistantInstallers.tsx"); mount(Component, { target: document.querySelector("#fixture"), props: { profile: "organize", notify: (message, error) => window.installationNotices.push({ message, error }) } });</script></body></html>',
}));
try {
  await page.goto(`${base}/__assistant-installers-test`);
  await expect(page.getByRole('heading', { name: 'Install in your assistants' })).toBeVisible();
  assert.deepEqual(await page.evaluate(() => window.installationCalls.map(call => call.command)), ['assistant_installation_status']);
  await page.getByRole('button', { name: 'Preview installation Codex', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Preview Codex', exact: true })).toContainText('C:\\Synthetic\\codex\\config');
  assert.equal(await page.evaluate(() => window.installationCalls.some(call => call.command === 'assistant_installation_apply')), false);
  await flip('Qamille skill');
  await expect(page.getByRole('switch', { name: 'Qamille skill' })).not.toBeChecked();
  await expect(page.getByRole('button', { name: 'Install', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Preview installation Codex', exact: true }).click();
  await page.getByRole('button', { name: 'Install', exact: true }).click();
  const applied = await page.evaluate(() => window.installationCalls.find(call => call.command === 'assistant_installation_apply'));
  assert.equal(applied.args.profile, 'organize');
  assert.equal(applied.args.installMcp, true);
  assert.equal(applied.args.installSkill, false);
  assert.equal(applied.args.token, 'preview-codex-organize');
  await expect(page.getByRole('button', { name: 'Install', exact: true })).toHaveCount(0);
  console.log('PASS install requires a fresh preview and preserves the selected profile/options');

  await flip('Qamille skill');
  await expect(page.getByRole('switch', { name: 'Qamille skill' })).toBeChecked();
  await flip('MCP connection');
  await expect(page.getByRole('switch', { name: 'MCP connection' })).not.toBeChecked();
  await expect(page.getByRole('button', { name: 'Preview installation Claude Desktop', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Preview installation Claude Cowork', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Preview installation Claude Cowork', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Prepare skill', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Prepare skill', exact: true }).click();
  await expect(page.getByText('C:\\Synthetic\\qamille.zip', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Open to finish', exact: true }).click();
  const opened = await page.evaluate(() => window.installationCalls.find(call => call.command === 'assistant_installation_open'));
  assert.deepEqual(opened.args, { target: 'claude_cowork' });
  assert.deepEqual(errors, []);
  console.log('PASS capability selection and Cowork package/import flow use only target-based commands');
} catch (error) {
  for (const diagnostic of diagnostics) console.error(diagnostic);
  throw error;
} finally {
  await browser.close();
}
