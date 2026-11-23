// The assistant, MCP and Google dialogs of the Compatto restyle (spec 2026-11-08 ark-compatto §7.2, Task 16
// part B): McpSettings (Select for the profile permission, Switch for every on/off option), the two
// switches of AssistantInstallers, the Assistant page (32px controls, 6px radius, borders and no shadows)
// and the two Google dialogs (Modal sm / md). Every component is mounted alone with synthetic IPC; no
// real account, client file or network is touched.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';

const base = process.env.QAMILLE_PREVIEW_URL ?? 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
const step = async (name, work) => {
  try {
    await work();
  } catch (error) {
    // A step that fails is told, and the next ones still run: one pass shows every difference from Sigrid, not the first.
    process.exitCode = 1;
    console.log(`FAIL STEP ${name}: ${String(error?.message ?? error).split('\n').slice(0, 8).join(' | ').slice(0, 700)}`);
    return;
  }
  console.log(`PASS ${name}`);
};

const stub = (script) =>
  `<!doctype html><html lang="en"><head><meta charset="utf-8"><link rel="stylesheet" href="/src/styles.css"></head><body><main id="fixture" style="max-width:720px;margin:auto"></main><script type="module">${script}</script></body></html>`;
const mountScript = (component, props) =>
  `const { mount } = await import("/tests/support/react-mount.js"); const { setLanguageChoice } = await import("/src/lib/i18n.ts"); setLanguageChoice("en"); const { default: Component } = await import("/src/components/${component}.tsx"); window.__props = ${props}; mount(Component, { target: document.querySelector("#fixture"), props: window.__props });`;

const open = async (route, body, { theme = 'light', size = { width: 1280, height: 900 }, init } = {}) => {
  const context = await browser.newContext({ locale: 'en-US', viewport: size, colorScheme: theme });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript(() => {
    window.isTauri = true;
    window.calls = [];
    window.notices = [];
  });
  if (init) await page.addInitScript(init);
  await page.route(`**/${route}`, (r) => r.fulfill({ contentType: 'text/html', body }));
  await page.goto(`${base}/${route}`);
  if (theme === 'dark') await page.evaluate(() => (document.documentElement.dataset.scheme = 'dark'));
  return page;
};
const css = (locator, property) =>
  locator.first().evaluate((node, name) => getComputedStyle(node).getPropertyValue(name), property);
const size = async (locator) => {
  const found = await locator.first().boundingBox();
  assert.ok(found, 'element has a box');
  return { width: Math.round(found.width), height: Math.round(found.height) };
};

// The computed value of `property` when it is set to var(token), so the test reads the token and not a copy of it.
const resolved = (page, property, token) =>
  page.evaluate(([name, value]) => {
    const probe = document.createElement('div');
    probe.style.setProperty(name, `var(${value})`);
    document.body.append(probe);
    const out = getComputedStyle(probe).getPropertyValue(name);
    probe.remove();
    return out;
  }, [property, token]);

// A click on the visible track of the switch whose label is `name` (the hidden input is under it).
const flip = (page, name) =>
  page.locator('[data-scope="switch"][data-part="root"]', { hasText: name }).locator('[data-part="control"]').click();

const mcpInit = () => {
  const limited = { access: 'ReadOnly', account_ids: [], calendar_ids: [], ai_execute: false, permanent_delete: false };
  let config = { profiles: { default: limited }, worker_autostart: false };
  window.__TAURI_INTERNALS__ = {
    async invoke(command, args = {}) {
      window.calls.push({ command, args: structuredClone(args) });
      if (command === 'mcp_configuration') return { config: structuredClone(config), worker_running: false, client_config: {} };
      if (command === 'assistant_installation_status')
        return { targets: [{ id: 'codex', name: 'Codex', mode: 'direct', detected: true, installed: false, supports_mcp: true, supports_skill: true }], executable_path: 'C:\\Synthetic\\Qamille.exe' };
      if (command === 'save_mcp_configuration') { config = structuredClone(args.config); return null; }
      throw new Error(`Unexpected command: ${command}`);
    },
  };
};
const mcpProps = `{ workspace: { settings: { mcp_access: "Send" }, accounts: [{ id: 1, email: "synthetic@example.test" }] }, notify: (message, error) => window.notices.push({ message, error }) }`;
const savedProfile = (page) =>
  page.evaluate(() => window.calls.filter((call) => call.command === 'save_mcp_configuration').at(-1)?.args.config.profiles.default);

const answer = { text: 'Synthetic answer about the Q4 proposal.', sources: [{ email_id: 1, passage: 'A synthetic passage long enough to be cut by the source chip of the assistant page' }], meeting_sources: [] };
const assistantInit = () => {
  window.__TAURI_INTERNALS__ = {
    async invoke(command, args = {}) {
      window.calls.push({ command, args: structuredClone(args) });
      if (command === 'ask_assistant') return structuredClone(window.__answer);
      throw new Error(`Unexpected command: ${command}`);
    },
  };
};

try {
  // ---------- McpSettings + AssistantInstallers ----------
  const mcpBody = stub(mountScript('McpSettings', mcpProps));
  await step('McpSettings: the profile permission is a Select (no native select), 32px, 6px radius', async () => {
    const page = await open('__c16b-mcp', mcpBody, { init: mcpInit });
    await expect(page.getByRole('button', { name: 'Save assistant permissions', exact: true })).toBeEnabled();
    assert.equal(await page.locator('select').count(), 0, 'no native <select>');
    const trigger = page.getByRole('combobox', { name: 'Profile permission', exact: true });
    await expect(trigger).toContainText('Read only');
    assert.equal((await size(trigger)).height, 32);
    assert.equal(await css(trigger, 'border-radius'), '6px');
    await trigger.click();
    await page.getByRole('option', { name: 'Send and remote calendar', exact: true }).click();
    await expect(trigger).toContainText('Send and remote calendar');
    await page.getByRole('button', { name: 'Save assistant permissions', exact: true }).click();
    assert.equal((await savedProfile(page)).access, 'Send');
    await page.context().close();
  });

  await step('McpSettings: its five on/off options and the two installer options are Switches, none a native checkbox', async () => {
    const page = await open('__c16b-mcp', mcpBody, { init: mcpInit });
    await expect(page.getByRole('button', { name: 'Save assistant permissions', exact: true })).toBeEnabled();
    assert.equal(await page.locator('input[type="checkbox"]:not([role="switch"])').count(), 0, 'no native checkbox');
    const names = ['All accounts', 'synthetic@example.test', 'All calendars', 'Allow AI processing of messages and meetings', 'Allow permanent deletion of drafts'];
    for (const name of names) await expect(page.getByRole('switch', { name, exact: true })).toBeVisible();
    await expect(page.getByRole('switch', { name: 'MCP connection', exact: true })).toBeChecked();
    await expect(page.getByRole('switch', { name: 'Qamille skill', exact: true })).toBeChecked();
    assert.equal(await page.getByRole('switch').count(), 7);
    const track = page.locator('[data-scope="switch"][data-part="control"]').first();
    {
      const { width, height } = await size(track);
      assert.ok(width > height, 'the switch track is wider than it is high');
    }
    await page.context().close();
  });

  await step('McpSettings: switches keep the permission semantics (all calendars and AI reach the saved profile; no worker at sign-in)', async () => {
    const page = await open('__c16b-mcp', mcpBody, { init: mcpInit });
    const save = page.getByRole('button', { name: 'Save assistant permissions', exact: true });
    await expect(save).toBeEnabled();
    const allCalendars = page.getByRole('switch', { name: 'All calendars', exact: true });
    await expect(allCalendars).not.toBeChecked();
    await expect(page.getByRole('textbox', { name: /Allowed calendars/ })).toBeVisible();
    await flip(page, 'All calendars');
    await expect(allCalendars).toBeChecked();
    await expect(page.getByRole('textbox', { name: /Allowed calendars/ })).toHaveCount(0);
    await flip(page, 'Allow AI processing of messages and meetings');
    await flip(page, 'synthetic@example.test');
    // Start at sign-in is Qamille in the tray now (Settings → General): the worker's switch is gone.
    await expect(page.getByRole('switch', { name: 'Run scheduled sends in the background at Windows sign-in', exact: true })).toHaveCount(0);
    await expect(page.getByText(/The worker is no longer needed at sign-in/)).toBeVisible();
    await save.click();
    const saved = await savedProfile(page);
    assert.equal(saved.calendar_ids, null);
    assert.equal(saved.ai_execute, true);
    assert.deepEqual(saved.account_ids, [1]);
    assert.equal(await page.evaluate(() => window.calls.filter((call) => call.command === 'save_mcp_configuration').at(-1).args.config.worker_autostart), false);
    await flip(page, 'All accounts');
    await expect(page.getByRole('switch', { name: 'synthetic@example.test', exact: true })).toHaveCount(0);
    await page.context().close();
  });

  await step('AssistantInstallers: the two options turn the preview selection off and on', async () => {
    const page = await open('__c16b-mcp', mcpBody, { init: mcpInit });
    const skill = page.getByRole('switch', { name: 'Qamille skill', exact: true });
    await expect(skill).toBeChecked();
    await flip(page, 'Qamille skill');
    await expect(skill).not.toBeChecked();
    await flip(page, 'MCP connection');
    await expect(page.getByText('Select at least one item to install.')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Preview installation Codex', exact: true })).toBeDisabled();
    await page.context().close();
  });

  await step('McpSettings in the dark theme: the switch track stays visible and the select keeps its border', async () => {
    const page = await open('__c16b-mcp', mcpBody, { init: mcpInit, theme: 'dark' });
    await expect(page.getByRole('button', { name: 'Save assistant permissions', exact: true })).toBeEnabled();
    const trigger = page.getByRole('combobox', { name: 'Profile permission', exact: true });
    assert.notEqual(await css(trigger, 'border-top-color'), await css(trigger, 'background-color'));
    assert.notEqual(await css(page.locator('[data-scope="switch"][data-part="control"]').first(), 'background-color'), await css(page.locator('body'), 'background-color'));
    await page.context().close();
  });

  // ---------- Assistant ----------
  const assistantBody = stub(`window.__answer = ${JSON.stringify(answer)};` + mountScript('Assistant', '{ openMail: (id) => window.calls.push({ command: "open-mail", id }), openMeeting: () => {} }'));
  await step('Assistant: 32px controls, 6px radius, borders and no shadow', async () => {
    const page = await open('__c16b-assistant', assistantBody, { init: assistantInit, size: { width: 1200, height: 800 } });
    const suggestions = page.locator('.assistant-welcome button');
    await expect(suggestions).toHaveCount(3);
    for (let i = 0; i < 3; i++) {
      const one = suggestions.nth(i);
      assert.equal((await size(one)).height, 32, 'suggestion is a 32px control');
      assert.equal(await css(one, 'border-radius'), '6px');
    }
    assert.equal(await css(page.locator('.assistant-symbol'), 'border-radius'), '6px');
    const form = page.locator('.ask-form');
    assert.equal(await css(form, 'border-radius'), '6px');
    assert.equal(await css(form, 'box-shadow'), 'none');
    assert.equal(await css(form, 'border-top-width'), '1px');
    const send = page.getByRole('button', { name: 'Ask assistant', exact: true });
    assert.deepEqual(await size(send), { width: 32, height: 32 });
    assert.equal(await css(page.getByRole('textbox', { name: 'Ask your assistant', exact: true }), 'background-color'), 'rgba(0, 0, 0, 0)');
    assert.equal(await css(page.getByRole('heading', { name: 'Your assistant', exact: true }), 'font-size'), await resolved(page, 'font-size', '--text-base'));
    assert.equal(await css(page.getByRole('heading', { name: 'Your assistant', exact: true }), 'font-weight'), '600');
    await page.context().close();
  });

  await step('Assistant: asking shows the answer, the source chips (4px radius) and still opens the mail', async () => {
    const page = await open('__c16b-assistant', assistantBody, { init: assistantInit, size: { width: 1200, height: 800 } });
    await page.getByRole('textbox', { name: 'Ask your assistant', exact: true }).fill('What about Q4?');
    await page.keyboard.press('Enter');
    await expect(page.getByText('Synthetic answer about the Q4 proposal.')).toBeVisible();
    assert.equal(await css(page.locator('.chat-question'), 'border-radius'), '6px');
    const chip = page.locator('.answer-sources button').first();
    await expect(chip).toBeVisible();
    assert.equal(await css(chip, 'border-radius'), '4px');
    assert.ok((await size(chip)).height <= 28, 'the source chip is lower than a control');
    await chip.click();
    assert.deepEqual(await page.evaluate(() => window.calls.at(-1)), { command: 'open-mail', id: 1 });
    await page.context().close();
  });

  await step('Assistant in the dark theme: the question bubble and the field edge read against the ground', async () => {
    const page = await open('__c16b-assistant', assistantBody, { init: assistantInit, theme: 'dark', size: { width: 1200, height: 800 } });
    await page.getByRole('textbox', { name: 'Ask your assistant', exact: true }).fill('What about Q4?');
    await page.keyboard.press('Enter');
    await expect(page.locator('.chat-question')).toBeVisible();
    assert.notEqual(await css(page.locator('.ask-form'), 'border-top-color'), await css(page.locator('.ask-form'), 'background-color'));
    await page.context().close();
  });

  // ---------- Google dialogs ----------
  const signIn = (props) => stub(mountScript('GoogleSignInDialog', `{ pending: true, error: "", hide: () => window.calls.push("hide"), cancel: () => window.calls.push("cancel"), retry: () => window.calls.push("retry"), setup: () => window.calls.push("setup"), ...${props} }`));
  await step('GoogleSignInDialog: a Modal sm (420px) with an 18px title and 32px actions', async () => {
    const page = await open('__c16b-signin', signIn('{}'));
    const dialog = page.getByRole('dialog', { name: 'Sign in with Google', exact: true });
    await expect(dialog).toBeVisible();
    assert.equal(await css(dialog, 'width'), '420px');
    assert.equal(await css(dialog, 'border-radius'), await resolved(page, 'border-radius', '--radius-lg'));
    assert.equal(await css(dialog.getByRole('heading'), 'font-size'), await resolved(page, 'font-size', '--text-xl'));
    for (const name of ['Cancel sign-in', 'Hide']) assert.equal((await size(dialog.getByRole('button', { name, exact: true }))).height, 32);
    await dialog.getByRole('button', { name: 'Hide', exact: true }).click();
    assert.deepEqual(await page.evaluate(() => window.calls), ['hide']);
    await page.context().close();
  });

  await step('GoogleSignInDialog: the failure shows the alert, the setup link as text and Try again as the primary action', async () => {
    const page = await open('__c16b-signin', signIn('{ pending: false, error: "Synthetic failure" }'));
    const dialog = page.getByRole('dialog', { name: 'Sign in with Google', exact: true });
    await expect(dialog.getByRole('alert')).toContainText('Synthetic failure');
    const retry = dialog.getByRole('button', { name: 'Try again', exact: true });
    assert.equal(await css(retry, 'background-color'), await resolved(page, 'background-color', '--color-primary'));
    await dialog.getByRole('button', { name: 'How to set up Google sign-in', exact: true }).click();
    assert.deepEqual(await page.evaluate(() => window.calls), ['setup']);
    await page.context().close();
  });

  const guideBody = stub(mountScript('GoogleSetupGuide', '{ close: () => window.calls.push("close"), configure: () => window.calls.push("configure") }'));
  await step('GoogleSetupGuide: a Modal md (560px) with an 18px title, five steps and 32px actions', async () => {
    const page = await open('__c16b-guide', guideBody, { size: { width: 1280, height: 1000 } });
    const dialog = page.getByRole('dialog', { name: 'How to set up Google sign-in', exact: true });
    await expect(dialog).toBeVisible();
    assert.equal(await css(dialog, 'width'), '560px');
    assert.equal(await css(dialog.getByRole('heading', { level: 2 }), 'font-size'), await resolved(page, 'font-size', '--text-xl'));
    assert.equal(await css(dialog.getByRole('heading', { level: 2 }), 'font-weight'), '600');
    await expect(dialog.locator('ol > li')).toHaveCount(5);
    const close = dialog.getByRole('button', { name: 'Close guide', exact: true });
    assert.deepEqual(await size(close), { width: 32, height: 32 });
    for (const name of ['Close', 'Enter credentials']) assert.equal((await size(dialog.getByRole('button', { name, exact: true }))).height, 32);
    await dialog.getByRole('button', { name: 'Enter credentials', exact: true }).click();
    await page.keyboard.press('Escape');
    assert.deepEqual(await page.evaluate(() => window.calls), ['configure', 'close']);
    await page.context().close();
  });

  await step('GoogleSetupGuide in the dark theme: step text keeps 4.5:1 against the surface', async () => {
    const page = await open('__c16b-guide', guideBody, { theme: 'dark', size: { width: 1280, height: 1000 } });
    const dialog = page.getByRole('dialog', { name: 'How to set up Google sign-in', exact: true });
    await expect(dialog).toBeVisible();
    const ratio = await dialog.evaluate((node) => {
      const parse = (value) => value.match(/[\d.]+/g).slice(0, 3).map(Number);
      const lum = ([r, g, b]) => { const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
      const text = getComputedStyle(node.querySelector('li p')).color;
      const ground = getComputedStyle(node).backgroundColor;
      const [a, b] = [lum(parse(text)), lum(parse(ground))].sort((x, y) => y - x);
      return (a + 0.05) / (b + 0.05);
    });
    assert.ok(ratio >= 4.5, `contrast ${ratio}`);
    await page.context().close();
  });

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
