// Real account settings with synthetic OAuth completion; never contacts Google.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:5187';
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1280, height: 900 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(() => {
  window.signInCalls = [];
  window.signInNotices = [];
  window.workspaceChanges = 0;
  if (window.location.search.includes('preview')) return;
  window.isTauri = true;
  window.__TAURI_INTERNALS__ = {
    invoke(command, args = {}) {
      window.signInCalls.push({ command, args: structuredClone(args) });
      if (command === 'open_link') return Promise.resolve(null);
      if (command === 'cancel_google_sign_in') {
        window.finishGoogleSignIn?.('Sign-in stopped.');
        return Promise.resolve(null);
      }
      if (command !== 'connect_google') return Promise.reject(new Error(`Unexpected command: ${command}`));
      return new Promise((resolve, reject) => {
        window.finishGoogleSignIn = message => message ? reject(new Error(message)) : resolve(null);
      });
    },
  };
});
await page.route('**/__google-sign-in-test*', route => route.fulfill({
  contentType: 'text/html',
  body: `<!doctype html><html><head><link rel="stylesheet" href="/src/styles.css"></head><body><main id="fixture"></main><script type="module">
    const { mount, unmount } = await import('/tests/support/react-mount.js');
    const { setLanguageChoice } = await import('/src/lib/i18n.ts');
    const { default: Component } = await import('/src/components/Settings.tsx');
    setLanguageChoice('en');
    const workspace = { accounts: [], counts: {}, labels: [], signatures: [], drafts: [], undo_actions: [],
      has_ai_key: false, has_google_secret: false,
      settings: { user_name: 'Synthetic', signature: '', ai_provider: 'Offline', ai_base_url: '', ai_model: '', auto_classify: true,
        auto_tag: true, mcp_access: 'Triage', dark: false, push_labels: false, google_client_id: '', time_zone: '', locale: '',
        undo_seconds: 10 } };
    const mountSettings = () => mount(Component, { target: document.querySelector('#fixture'), props: { workspace,
      changed: () => window.workspaceChanges++, notify: (message, error) => window.signInNotices.push({ message, error: !!error }) } });
    let component = mountSettings();
    window.remountSettings = async () => { await unmount(component); component = mountSettings(); };
  </script></body></html>`,
}));

const dialog = () => page.getByRole('dialog', { name: 'Sign in with Google', exact: true });
const signIn = () => page.getByRole('button', { name: 'Sign in with Google', exact: true });
const calls = () => page.evaluate(() => window.signInCalls.map(call => call.command));
async function openAccounts(query = '') {
  await page.goto(`${base}/__google-sign-in-test${query}`);
  await page.getByRole('button', { name: 'Accounts', exact: true }).click();
}

try {
  await openAccounts();
  await expect(signIn()).toBeVisible({ timeout: 3000 });
  await expect(page.getByRole('textbox', { name: 'Google desktop client ID', exact: true })).toBeHidden();
  await expect(page.getByLabel('Google client secret', { exact: true })).toBeHidden();
  await signIn().click();
  await expect(dialog()).toBeVisible();
  await expect(dialog()).toContainText(/browser/i);
  assert.deepEqual(await calls(), ['connect_google'], 'Sign-in does not require saving settings');
  await dialog().getByRole('button', { name: 'Hide', exact: true }).click();
  await expect(dialog()).toBeHidden();
  // Closing with a button must not drop the focus on <body> (Modal returns it to the opener).
  assert.notEqual(await page.evaluate(() => document.activeElement?.tagName), 'BODY', 'focus is not lost when the dialog closes');
  const progress =page.getByRole('button', { name: 'Show sign-in progress', exact: true });
  await progress.click();
  await expect(dialog()).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog()).toBeHidden();
  await progress.click();
  assert.deepEqual(await calls(), ['connect_google'], 'Hiding and reopening must not launch another OAuth request');
  await page.evaluate(() => window.finishGoogleSignIn());
  await expect(dialog()).toBeHidden();
  await expect(signIn()).toBeEnabled();
  assert.equal(await page.evaluate(() => window.workspaceChanges), 1);
  assert.deepEqual(await page.evaluate(() => window.signInNotices), [{ message: 'Google account connected', error: false }]);
  console.log('PASS direct Google sign-in, waiting dialog, hide/reopen and successful refresh');

  await openAccounts('?failure');
  await signIn().click();
  await page.evaluate(() => window.finishGoogleSignIn('Synthetic Google sign-in failed'));
  await expect(dialog().getByRole('alert')).toContainText('Synthetic Google sign-in failed');
  await dialog().getByRole('button', { name: 'How to set up Google sign-in', exact: true }).click();
  await expect(dialog()).toBeHidden();
  const setup = page.getByRole('dialog', { name: 'How to set up Google sign-in', exact: true });
  await expect(setup).toBeVisible();
  await expect(setup.locator('ol > li')).toHaveCount(5);
  const projectUrl = 'https://console.cloud.google.com/projectcreate';
  await setup.locator(`a[href="${projectUrl}"]`).click();
  assert.deepEqual(await page.evaluate(() => window.signInCalls.at(-1)), {
    command: 'open_link', args: { url: projectUrl },
  });
  await setup.getByRole('button', { name: 'Enter credentials', exact: true }).click();
  await expect(setup).toBeHidden();
  await expect(page.getByRole('textbox', { name: 'Google desktop client ID', exact: true })).toBeVisible();
  await expect(page.getByLabel('Google client secret', { exact: true })).toBeVisible();
  assert.deepEqual(await calls(), ['connect_google', 'open_link'], 'Setup guide must not save or start sign-in automatically');
  console.log('PASS failed sign-in links to five-step setup guide and credential entry');

  await openAccounts('?retry');
  await signIn().click();
  await page.evaluate(() => window.finishGoogleSignIn('Synthetic Google sign-in failed'));
  await expect(dialog().getByRole('alert')).toContainText('Synthetic Google sign-in failed');
  await dialog().getByRole('button', { name: 'Try again', exact: true }).click();
  assert.deepEqual(await calls(), ['connect_google', 'connect_google']);
  await expect(dialog().getByRole('alert')).toHaveCount(0);
  await page.evaluate(() => window.finishGoogleSignIn());
  await expect(dialog()).toBeHidden();
  assert.equal(await page.evaluate(() => window.workspaceChanges), 1);
  console.log('PASS failed sign-in remains actionable and retry completes once');

  await openAccounts('?guide');
  await page.getByRole('button', { name: 'How to set up Google sign-in', exact: true }).click();
  await expect(setup).toBeVisible();
  await expect(setup.locator('ol > li')).toHaveCount(5);
  await expect(setup.locator(`a[href="${projectUrl}"]`)).toBeVisible();
  await expect(setup.locator('a[href="https://developers.google.com/identity/protocols/oauth2/native-app"]')).toBeVisible();
  assert.deepEqual(await calls(), [], 'Opening setup does not launch OAuth or save settings');
  await setup.getByRole('button', { name: 'Enter credentials', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Google desktop client ID', exact: true })).toBeVisible();
  assert.deepEqual(await calls(), []);
  console.log('PASS setup is accessible directly from Accounts without starting sign-in');

  await openAccounts('?remount');
  await signIn().click();
  await dialog().getByRole('button', { name: 'Hide', exact: true }).click();
  await page.evaluate(() => window.remountSettings());
  await page.getByRole('button', { name: 'Accounts', exact: true }).click();
  await signIn().click();
  await expect(dialog()).toBeVisible();
  assert.deepEqual(await calls(), ['connect_google'], 'Returning to Settings must reuse the pending OAuth request');
  await page.evaluate(() => window.finishGoogleSignIn());
  await expect(dialog()).toBeHidden();
  await expect(signIn()).toBeEnabled();
  assert.equal(await page.evaluate(() => window.workspaceChanges), 1, 'A pending sign-in completed after remount refreshes the workspace once');
  console.log('PASS returning to Settings reuses the pending sign-in without opening another browser');

  await openAccounts('?cancel');
  await signIn().click();
  await dialog().getByRole('button', { name: 'Cancel sign-in', exact: true }).click();
  await expect(dialog()).toBeHidden();
  await expect(signIn()).toBeEnabled();
  assert.deepEqual(await calls(), ['connect_google', 'cancel_google_sign_in']);
  assert.deepEqual(await page.evaluate(() => window.signInNotices), [], 'A cancelled sign-in is not an error');
  assert.equal(await page.evaluate(() => window.workspaceChanges), 0);
  await signIn().click();
  assert.deepEqual(await calls(), ['connect_google', 'cancel_google_sign_in', 'connect_google'], 'After cancelling, a new sign-in can start');
  await page.evaluate(() => window.finishGoogleSignIn());
  await expect(dialog()).toBeHidden();
  console.log('PASS cancelling a waiting sign-in closes it quietly and a new one can start');

  await openAccounts('?italian');
  await page.evaluate(async () => {
    const { setLanguageChoice } = await import('/src/lib/i18n.ts');
    setLanguageChoice('it');
  });
  await page.getByRole('button', { name: "Come attivare l'accesso Google", exact: true }).click();
  const italianGuide = page.getByRole('dialog', { name: "Come attivare l'accesso Google", exact: true });
  await expect(italianGuide).toBeVisible();
  await expect(italianGuide.locator('ol > li')).toHaveCount(5);
  await expect(italianGuide).toContainText('Crea il progetto Qamille');
  await expect(italianGuide.getByRole('button', { name: 'Inserisci credenziali', exact: true })).toBeVisible();
  await page.screenshot({ path: '../checks/google-setup-guide.png' });
  console.log('PASS Italian setup guide renders; screenshot saved to checks/google-setup-guide.png');

  await openAccounts('?preview');
  await signIn().click();
  await expect(dialog().getByRole('alert')).toContainText(/desktop app|preview/i);
  assert.equal(await page.evaluate(() => window.workspaceChanges), 0);
  assert.equal(await page.evaluate(() => window.signInNotices.some(notice => !notice.error)), false);
  assert.deepEqual(await calls(), []);
  assert.deepEqual(errors, []);
  console.log('PASS browser preview cannot report a real account connected');
} finally {
  await browser.close();
}
