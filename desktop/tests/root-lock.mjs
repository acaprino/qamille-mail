// The root of every WebView does not scroll past the edge (docs/aspetto-nativo.md rule 4, audit Q5): a precision
// touchpad or a touchscreen must never slide the whole view and uncover the window background. A screen taller
// than the window scrolls inside its own container; the message frame hands the wheel to the reader.
// Synthetic data only. Run: npm run test:browser -- root-lock   (from desktop/)
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { previewUrl } from './support/compatto.mjs';

// 1000x640 at 200 %: the app zoom scales CSS pixels and Playwright has no app zoom, so the page sees 500x320.
const ZOOMED = { width: 500, height: 320 };
const roomy = { width: 1440, height: 900 };

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const errors = [];
async function start({ init, size = roomy } = {}) {
  const context = await browser.newContext({ locale: 'en-US', viewport: size });
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on('pageerror', (error) => errors.push(error.message));
  if (init) await page.addInitScript(init.script, init.arg);
  await page.goto(previewUrl);
  return { page, context };
}

// What the root can do: nothing to scroll, and a wheel or a script cannot move it.
async function assertRootFixed(page, at) {
  const lock = await page.evaluate(() => {
    const html = getComputedStyle(document.documentElement);
    return { overscroll: `${html.overscrollBehaviorX} ${html.overscrollBehaviorY}`, html: html.overflow, body: getComputedStyle(document.body).overflow };
  });
  assert.deepEqual(lock, { overscroll: 'none none', html: 'hidden', body: 'hidden' }, `${at}: html and body are locked (overscroll-behavior: none, overflow: hidden)`);
  const before = await page.evaluate(() => {
    const el = document.scrollingElement;
    return { scrollHeight: el.scrollHeight, clientHeight: el.clientHeight, scrollWidth: el.scrollWidth, clientWidth: el.clientWidth };
  });
  assert.equal(before.scrollHeight, before.clientHeight, `${at}: the document is exactly one window tall (${JSON.stringify(before)})`);
  assert.equal(before.scrollWidth, before.clientWidth, `${at}: the document is exactly one window wide (${JSON.stringify(before)})`);
  const { width, height } = page.viewportSize();
  for (const [x, y] of [[width / 2, height / 2], [width - 6, height / 2], [width / 2, 6], [width / 2, height - 6]]) {
    await page.mouse.move(x, y);
    await page.mouse.wheel(0, 900);
    await page.mouse.wheel(0, -900);
    await page.mouse.wheel(900, 0);
  }
  const after = await page.evaluate(() => {
    window.scrollTo(0, 5000);
    window.scrollTo(5000, 0);
    return { top: document.scrollingElement.scrollTop, left: document.scrollingElement.scrollLeft };
  });
  assert.deepEqual(after, { top: 0, left: 0 }, `${at}: wheel and scripts leave the root where it is`);
}

try {
  // Mail and Settings at the smallest window at 200 %.
  {
    const { page, context } = await start();
    await page.locator('.compose-main').waitFor();
    await page.setViewportSize(ZOOMED);
    await assertRootFixed(page, 'mail, 1000x640 at 200 %');
    console.log('PASS the root of the mail screen stays put at 1000x640, 200 %');

    await page.setViewportSize(roomy);
    await page.locator('.sidebar .profile').click();
    await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
    await page.setViewportSize(ZOOMED);
    await assertRootFixed(page, 'settings, 1000x640 at 200 %');
    await page.setViewportSize(roomy);
    await assertRootFixed(page, 'settings, 1440x900');
    console.log('PASS the root of Settings stays put at 1000x640, 200 % and at 1440x900');
    await context.close();
  }

  // The startup screen: while the workspace opens, and when it could not open.
  for (const [name, hook] of [
    ['opening', { __previewCommandDelay: { workspace_snapshot: 120000 } }],
    // A long message: at 500x320 the failed screen is taller than the window, and its top must stay reachable.
    ['failed', { __previewCommandFail: { workspace_snapshot: 'Synthetic failure of the preview workspace. '.repeat(40) } }],
  ]) {
    const { page, context } = await start({
      size: ZOOMED,
      init: { script: (value) => Object.assign(window, value), arg: hook },
    });
    const screen = page.locator('.startup-state');
    await expect(screen).toBeVisible();
    await assertRootFixed(page, `startup screen (${name}), 1000x640 at 200 %`);
    // The screen is the scroller: the whole of it can be reached, top and foot, and its wheel stays inside it.
    const reach = await screen.evaluate((node) => {
      const kids = [...node.children];
      node.scrollTop = 0;
      const top = kids[0].getBoundingClientRect().top;
      node.scrollTop = node.scrollHeight;
      const foot = kids[kids.length - 1].getBoundingClientRect().bottom;
      return { top, foot, view: innerHeight, taller: node.scrollHeight > node.clientHeight, overflow: getComputedStyle(node).overflowY, contain: getComputedStyle(node).overscrollBehaviorY };
    });
    if (name === 'failed') assert.ok(reach.taller, 'startup (failed): the screen is taller than the window, so it scrolls');
    assert.ok(reach.top >= 0, `startup (${name}): the first line is reachable (top ${reach.top})`);
    assert.ok(reach.foot <= reach.view + 0.5, `startup (${name}): the last line is reachable (foot ${reach.foot} of ${reach.view})`);
    assert.equal(reach.overflow, 'auto', `startup (${name}): the screen scrolls inside itself`);
    assert.equal(reach.contain, 'contain', `startup (${name}): its overscroll stays inside it`);
    console.log(`PASS the startup screen (${name}) scrolls inside itself, the root stays put`);
    await context.close();
  }

  // The sign-in screen of the Professional build scrolls down to its last button.
  {
    const { page, context } = await start({
      size: ZOOMED,
      init: { script: (value) => (window.__previewServer = value), arg: { edition: 'professional' } },
    });
    await expect(page.getByRole('heading', { name: 'Sign in to Qamille' })).toBeVisible();
    await expect(page.getByText('The server is ready.')).toBeVisible();
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    await expect(page.getByLabel('Password', { exact: true })).toBeVisible();
    await assertRootFixed(page, 'sign-in form, 1000x640 at 200 %');
    const screen = page.locator('.onboarding');
    const sizes = await screen.evaluate((node) => ({ scroll: node.scrollHeight, client: node.clientHeight }));
    assert.ok(sizes.scroll > sizes.client, `the sign-in form is taller than the window, so it scrolls (${JSON.stringify(sizes)})`);
    const last = screen.getByRole('button').last();
    await page.mouse.move(ZOOMED.width / 2, ZOOMED.height / 2);
    await page.mouse.wheel(0, 2000);
    await expect.poll(() => screen.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
    await expect(last).toBeInViewport({ ratio: 1 });
    // At the end of the screen the wheel does not carry on into the root.
    await page.mouse.wheel(0, 2000);
    assert.equal(await page.evaluate(() => document.scrollingElement.scrollTop), 0, 'the root does not follow the screen');
    // And back up: the first line of the card is reachable too.
    await page.mouse.wheel(0, -4000);
    await expect.poll(() => screen.evaluate((node) => node.scrollTop)).toBe(0);
    await expect(page.getByRole('heading', { name: 'Create your account' }).or(page.getByRole('heading', { name: 'Sign in to Qamille' }))).toBeInViewport();
    console.log('PASS the sign-in screen scrolls to its last button and back, the root stays put');
    await context.close();
  }

  // The wheel over the frame of a long HTML message scrolls the reader (the frame never scrolls itself).
  {
    const sample = JSON.parse(fs.readFileSync(path.resolve(import.meta.dirname, '../src/lib/sample.json'), 'utf8'));
    const newsletter = sample.emails.find((email) => email.subject === 'This week: 5 tools for focused work');
    assert.ok(newsletter, 'the sample has the newsletter');
    const { page, context } = await start({
      init: { script: () => localStorage.setItem('qamille:mail-display', 'threads') },
    });
    await page.locator('.mail-row').first().waitFor();
    const lines = Array.from({ length: 60 }, (_, i) => `<p>Line ${i + 1} of a long issue, with enough words to fill the width of the reading pane.</p>`).join('');
    await page.evaluate(([id, html]) => window.__previewEdit(id, { body_html: html }), [newsletter.id, `<html><body><h1>A long issue</h1>${lines}</body></html>`]);
    const closed = page.locator('.group-heading[aria-expanded="false"]');
    while (await closed.count()) await closed.first().click();
    await page.getByText('This week: 5 tools for focused work').first().click();
    const frame = page.locator('.html-body iframe');
    await expect(page.frameLocator('.html-body iframe').locator('h1')).toHaveText('A long issue');
    const reader = page.locator('.reader-scroll');
    const room = await reader.evaluate((node) => ({ scroll: node.scrollHeight, client: node.clientHeight, top: node.scrollTop }));
    assert.ok(room.scroll > room.client, `the long message is taller than the reader (${JSON.stringify(room)})`);
    assert.equal(
      await frame.evaluate((node) => getComputedStyle(node.contentDocument.documentElement).overscrollBehaviorY),
      'auto',
      'the frame keeps the default overscroll-behavior, so the wheel chains to the reader',
    );
    const box = await frame.boundingBox();
    await page.mouse.move(box.x + box.width / 2, Math.min(box.y + 200, 400));
    await page.mouse.wheel(0, 500);
    await expect.poll(() => reader.evaluate((node) => node.scrollTop)).toBeGreaterThan(room.top);
    await assertRootFixed(page, 'mail with a long HTML message');
    console.log('PASS the wheel over the frame of a long HTML message scrolls the reader');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
