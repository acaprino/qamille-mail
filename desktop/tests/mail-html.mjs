// HTML mail renders sanitized in an isolated frame, with remote images blocked until allowed.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { openAllGroups } from './support/groups.mjs';

const port = 28163;
const url = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: path.resolve(import.meta.dirname, '..'), windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', data => output += data);
server.stderr.on('data', data => output += data);
let browser;
try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 1000 }, colorScheme: 'dark' });
  const errors = [];
  const remoteRequests = [];
  page.on('pageerror', error => errors.push(error.message));
  // A request reaches the network only when the route sees it; CSP-blocked loads never do.
  await page.route('https://images.example.com/**', route => {
    remoteRequests.push(route.request().url());
    return route.fulfill({ status: 404, body: '' });
  });
  if (process.env.DEBUG_HTML) page.on('requestfailed', r => console.log('FAILED', r.url(), r.failure()?.errorText));
  await page.addInitScript(() => {
    localStorage.setItem('qamille:mail-display', 'threads');
    window.openedLinks = [];
  });
  await page.route(/\/src\/lib\/preview\.ts(?:\?|$)/, async route => {
    const response = await route.fetch();
    const source = await response.text();
    assert.match(source, /export async function previewCommand\(/);
    await route.fulfill({ response, body: source.replace('export async function previewCommand(', 'async function originalPreviewCommand(') + `
      export async function previewCommand(command, args = {}) {
        if (command === 'open_link') { window.openedLinks.push(args.url); return; }
        if (command === 'save_draft' && window.failNextSave) { window.failNextSave = false; throw new Error('The disk is full.'); }
        return originalPreviewCommand(command, args);
      }
    ` });
  });
  await page.goto(url);
  await expect(page.locator('.message-body')).toBeVisible();
  // The newsletter also holds every other kind of link a message can have: an image map, an SVG link
  // (xlink:href), a link to a later part of the message, and the anchor another one names with name=.
  const sample = JSON.parse(fs.readFileSync(path.resolve(import.meta.dirname, '../src/lib/sample.json'), 'utf8'));
  const newsletter = sample.emails.find(email => email.id === 8);
  assert.ok(newsletter.body_html.includes('</div></body>'));
  const gif = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
  const links = `<a name="legacy"></a><p><a id="jump" href="#part2">Jump to part 2</a></p>
    <p><a id="spaced" href=" https://example.com/spaced ">Spaced</a> <a id="nbsp" href="&nbsp;https://example.com/x">No-break space</a>
    <a id="bom" href="&#xFEFF;https://example.com/x">BOM</a> <a id="ideographic" href="&#x3000;https://example.com/x">Ideographic space</a>
    <a id="nohost" href="http:foo">No slashes</a> <a id="oneslash" href="HTTP:/foo">One slash</a> <a id="relative" href="/relative">Relative</a>
    <a id="data" href="data:text/html,x">Data</a> <a id="mail" href="mailto:a@b.it?subject=x">Write to us</a></p>
    <map name="zones"><area shape="rect" coords="0,0,100,100" href="https://example.com/a" alt="Zone"></map>
    <img usemap="#zones" src="${gif}" width="100" height="100" alt="">
    <svg width="120" height="40" xmlns:xlink="http://www.w3.org/1999/xlink"><a xlink:href="https://example.com/b"><rect width="120" height="40" fill="#c00"/></a></svg>
    <div style="height:1400px">Space</div>
    <h2 id="part2">Part 2</h2><p><a id="back" href="#legacy">Back to the top</a></p>`;
  await page.evaluate(([id, html]) => window.__previewEdit(id, { body_html: html }),
    [8, newsletter.body_html.replace('</div></body>', `${links}</div></body>`)]);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  // A plain-text message keeps the text reader.
  await expect(page.locator('article.message-body')).toBeVisible();

  // The newsletter lives in a quiet group, which starts closed.
  await openAllGroups(page);
  await page.getByText('This week: 5 tools for focused work').first().click();
  const frame = page.frameLocator('.html-body iframe');
  await expect(frame.locator('h1')).toHaveText('5 tools for focused work');
  await expect(page.locator('article.message-body')).toHaveCount(0);

  const inside = await page.locator('.html-body iframe').evaluate(element => {
    const doc = element.contentDocument;
    return {
      sandbox: element.getAttribute('sandbox'),
      scripts: doc.querySelectorAll('script').length,
      ran: doc.body.dataset.ran ?? null,
      policy: doc.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute('content'),
      styled: getComputedStyle(doc.querySelector('.card')).borderRadius,
      height: element.getBoundingClientRect().height,
      content: doc.documentElement.scrollHeight,
    };
  });
  assert.equal(inside.sandbox, 'allow-same-origin', 'Scripts are never allowed in the frame');
  assert.equal(inside.scripts, 0, 'Sanitizing removes scripts');
  assert.equal(inside.ran, null, 'No script ran');
  assert.match(inside.policy, /img-src data: cid:;/, 'Remote images are blocked by default');
  assert.equal(inside.styled, '12px', 'The sender stylesheet applies');
  assert.ok(Math.abs(inside.height - inside.content) <= 2, `Frame fits its content: ${JSON.stringify(inside)}`);
  assert.deepEqual(remoteRequests, [], 'No remote image is requested before consent');
  const darkEmail = await page.locator('.html-body iframe').evaluate(element => {
    const doc = element.contentDocument;
    return {
      theme: doc.documentElement.dataset.theme,
      filter: getComputedStyle(doc.documentElement).filter,
      imageFilter: getComputedStyle(doc.querySelector('img')).filter,
    };
  });
  assert.equal(darkEmail.theme, 'dark', 'The email follows the app dark theme');
  assert.notEqual(darkEmail.filter, 'none', 'Dark mode transforms email colors');
  assert.notEqual(darkEmail.imageFilter, 'none', 'Images are counter-transformed and keep their colors');

  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expect(frame.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByRole('button', { name: 'Toggle dark appearance' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(frame.locator('html')).toHaveAttribute('data-theme', 'dark');
  assert.equal(await page.evaluate(() => localStorage.getItem('qamille:appearance')), 'dark');

  await page.getByRole('button', { name: 'Alex Personal workspace' }).click();
  await page.getByRole('combobox', { name: 'Appearance', exact: true }).click();
  await page.getByRole('option', { name: 'Automatic (system)', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  await page.getByRole('button', { name: 'Inbox', exact: false }).first().click();
  await page.getByText('This week: 5 tools for focused work').first().click();

  // A mail link of a message opens Qamille's composer with the link's fields, never another mail
  // program: nothing reaches open_link, and the frame keeps its document (rule 32, audit Q10).
  // A draft whose fields could not be saved is not left empty in Drafts: the failure is said, and no
  // composer opens.
  const draftCount = () => page.evaluate(async () => (await (await import('/src/lib/api.ts')).call('list_drafts')).length);
  const draftsBefore = await draftCount();
  await page.evaluate(() => { window.failNextSave = true; });
  await frame.locator('#mail').click();
  await expect(page.getByText('The disk is full.')).toBeVisible();
  await expect.poll(draftCount).toBe(draftsBefore);
  const composer = page.getByRole('tabpanel');
  await expect(composer).toHaveCount(0);
  await frame.locator('#mail').click();
  await expect(composer.getByRole('textbox', { name: 'Subject', exact: true })).toHaveValue('x');
  await expect(composer.getByRole('button', { name: 'Remove a@b.it', exact: true })).toBeVisible();
  await expect(composer.locator('[data-scope="tags-input"][data-part="item"]')).toHaveCount(1);
  await expect(frame.locator('h1')).toHaveText('5 tools for focused work');
  assert.deepEqual(await page.evaluate(() => window.openedLinks), [], 'A mail link never reaches open_link');
  await composer.getByRole('button', { name: 'Save and close draft' }).click();
  await expect(composer).toHaveCount(0);

  await frame.getByRole('link', { name: 'Read the issue' }).click();
  await expect.poll(() => page.evaluate(() => window.openedLinks)).toEqual(['https://focus.example.com/issue']);
  await expect(frame.locator('h1')).toBeVisible();

  // Every other link leaves through the browser too, and none of them navigates the frame: it keeps
  // its document (a navigation would end in the browser's error page, chrome-error).
  const iframe = page.locator('.html-body iframe');
  const frameAddress = () => page.frames().filter(f => f !== page.mainFrame()).map(f => f.url());
  await iframe.evaluate(element => { element.contentDocument.__kept = true; });
  const address = frameAddress();
  const kept = () => iframe.evaluate(element => element.contentDocument?.__kept === true);
  await frame.locator('img[usemap]').click({ position: { x: 30, y: 30 } });
  await expect.poll(() => page.evaluate(() => window.openedLinks.slice(1))).toEqual(['https://example.com/a']);
  await frame.locator('svg a').click();
  await expect.poll(() => page.evaluate(() => window.openedLinks.slice(1))).toEqual(['https://example.com/a', 'https://example.com/b']);
  assert.equal(await kept(), true, 'An image map or SVG link must not navigate the frame');
  assert.deepEqual(frameAddress(), address, 'The frame keeps its address');

  // A link to a part of the message scrolls to it, by id or by the name of an anchor, and opens nothing.
  const placeOf = id => iframe.evaluate((element, target) => Math.round(
    element.getBoundingClientRect().top + element.contentDocument.getElementById(target).getBoundingClientRect().top), id);
  const viewport = await page.evaluate(() => window.innerHeight);
  const onScreen = async id => { const top = await placeOf(id); return top >= 0 && top < viewport; };
  assert.ok(await placeOf('part2') > viewport, 'Part 2 starts below what is on screen');
  await frame.locator('#jump').click();
  await expect.poll(() => onScreen('part2')).toBe(true);
  await frame.locator('#back').click();
  await expect.poll(() => onScreen('jump')).toBe(true);
  assert.equal(await kept(), true, 'A link to a part of the message must not navigate the frame');
  assert.deepEqual(frameAddress(), address, 'The frame keeps its address');
  assert.equal(await page.evaluate(() => window.openedLinks.length), 3, 'A link to a part of the message opens nothing');

  // The system browser gets an absolute web or mail address the message wrote, never an address that only
  // resolves against the app's own (http://tauri.localhost/...): a relative link, a scheme without `//`
  // (http:foo, HTTP:/foo) or a leading no-break space, BOM or ideographic space (DOMPurify trims those from
  // an attribute; externalAddress itself is checked in native-feel.mjs) must not reach open_link as one.
  for (const id of ['spaced', 'nbsp', 'bom', 'ideographic', 'nohost', 'oneslash', 'relative', 'data']) {
    await frame.locator(`#${id}`).click();
    assert.equal(await kept(), true, `#${id} must not navigate the frame`);
  }
  // Each click is handled before the next one: a link that did open something has shown by now.
  await page.waitForTimeout(300);
  const opened = await page.evaluate(() => window.openedLinks.slice(3));
  assert.equal(opened[0], 'https://example.com/spaced', 'A leading space is dropped, as the URL parser does');
  assert.ok(opened.every(link => /^https:\/\/example\.com\//.test(link)), `Only absolute addresses of the message open: ${opened}`);
  assert.ok(!opened.some(link => link.startsWith(url)), 'No address of the app itself reaches the browser');
  assert.deepEqual(frameAddress(), address, 'The frame keeps its address');

  // Keys pressed in the message body reach the app's shortcuts; the reading keys stay in the frame.
  await frame.locator('h1').click();
  await page.keyboard.press('ArrowDown');
  await expect(frame.locator('h1')).toHaveText('5 tools for focused work');
  await page.keyboard.press('Control+k');
  await expect(page.locator('.global-search input')).toBeFocused();

  await page.getByRole('button', { name: 'Show images' }).click();
  await expect(page.getByRole('button', { name: 'Show images' })).toHaveCount(0);
  await expect.poll(() => remoteRequests.length).toBeGreaterThan(0);
  const allowed = await page.locator('.html-body iframe').evaluate(element =>
    element.contentDocument.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute('content'));
  assert.match(allowed, /img-src data: cid: https: http:/);

  // E pressed in the message body archives it, as anywhere else outside a field.
  await frame.locator('h1').click();
  const archived = () => page.evaluate(() => (window.__previewCalls ?? []).filter((c) => c === 'mail_action' || c === 'queue_mail_action').length);
  const before = await archived();
  await page.keyboard.press('e');
  await expect.poll(archived).toBeGreaterThan(before);

  assert.deepEqual(errors, []);
  console.log('mail-html: all checks passed');
} finally {
  await browser?.close();
  server.kill();
}
