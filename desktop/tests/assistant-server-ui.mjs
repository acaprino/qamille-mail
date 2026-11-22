// Gateway LLM, wave 5: the assistant «Server Qamille» in the interface. The consent step names
// the provider the server declares (INV-34); Settings → Assistant offers «Server Qamille» first
// when the connected server has an assistant, asks for a new yes when the provider changed, and
// shows no key fields for it.
// Run against the browser preview: QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
import { chromium } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
async function open(server) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.addInitScript((value) => (window.__previewServer = value), server);
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.getByRole('button', { name: /^New message/ }).waitFor();
  await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
  return { page, context };
}

try {
  // The consent step says where the assistant's requests go.
  {
    const { page, context } = await open({ llm: { external: true, label: 'OpenAI' } });
    await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
    await page.getByRole('button', { name: 'Connect to a server', exact: true }).click();
    await page.getByLabel(/Server address|Indirizzo/i).fill('https://calcolo.ufficio.it');
    await page.getByText(/Qamille server found/).waitFor();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.getByText('Assistant requests are forwarded to OpenAI, a service outside this server.').waitFor();
    await context.close();
  }
  {
    const { page, context } = await open({ llm: { external: false, label: 'Server' } });
    await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
    await page.getByRole('button', { name: 'Connect to a server', exact: true }).click();
    await page.getByLabel(/Server address|Indirizzo/i).fill('https://calcolo.ufficio.it');
    await page.getByText(/Qamille server found/).waitFor();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.getByText('The assistant uses a model that runs on this server.').waitFor();
    await context.close();
  }
  // Connected to a server with an assistant: «Qamille server» first, no key fields.
  {
    const { page, context } = await open({ connected: true, llm: { external: false, label: 'Server' } });
    await page.locator('.settings-nav').getByRole('button', { name: 'Assistant', exact: true }).click();
    await page.getByRole('combobox', { name: 'Provider' }).click();
    const options = await page.getByRole('option').allTextContents();
    assert.equal(options[0], 'Qamille server');
    assert.ok(options.some((text) => text.startsWith('Use your own key')), options.join('|'));
    await page.getByRole('option', { name: 'Qamille server' }).click();
    assert.equal(await page.getByLabel('API key').count(), 0, 'no key for the server provider');
    assert.equal(await page.getByRole('alert').count(), 0, 'no warning when the consent is current');
    await context.close();
  }
  // The server changed its provider: a warning and «Confirm».
  {
    const { page, context } = await open({ connected: true, llm: { external: true, label: 'OpenAI' }, llmChanged: true });
    await page.locator('.settings-nav').getByRole('button', { name: 'Assistant', exact: true }).click();
    await page.getByText(/now uses another provider \(OpenAI, an external service\): confirm to continue/).waitFor();
    await page.getByRole('button', { name: 'Confirm', exact: true }).click();
    await context.close();
  }
  // Not connected: the old provider list, without «Qamille server».
  {
    const { page, context } = await open({});
    await page.locator('.settings-nav').getByRole('button', { name: 'Assistant', exact: true }).click();
    await page.getByRole('combobox', { name: 'Provider' }).click();
    const options = await page.getByRole('option').allTextContents();
    assert.ok(!options.includes('Qamille server'), options.join('|'));
    await context.close();
  }
  assert.deepEqual(errors, []);
  console.log('assistant-server-ui: ok');
} finally {
  await browser.close();
}
