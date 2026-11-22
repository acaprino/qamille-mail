// The dark theme of the Compatto look (spec 2026-11-08 ark-compatto, sections 6 and 7.1; Task 11).
//   1. File: every colour token of the light block has a dark value of its own.
//   2. The real app with data-scheme="dark": the sidebar is darker than the sheet, the message body reads at 4.5:1.
//   3. The new primitives through the gallery (button, select, checkbox, switch, tabs, slider, date field with
//      its calendar, modal, popover, side panel) in the dark: the focus ring is 2px in the accent, every
//      visible text reads at 4.5:1 over what is really painted behind it, no surface is transparent or white
//      on white, and a field rests on --border-strong (ruling R16).
// Synthetic data only. Run with the preview address in QAMILLE_PREVIEW_URL (npm run test:browser sets it).
//   node scripts/test-browser.mjs compatto-dark
import assert from 'node:assert/strict';
import { openApp } from './support/compatto.mjs';
import { mountGallery } from './support/mount.mjs';
import { readThemes } from './support/theme-tokens.mjs';

// ---- 1. every colour token of the light block is redeclared in the dark block -------------------------------
// Exempt: the paper of the printed message and of the document views (it stays white on purpose), the pastel
// sender colours (they carry the dark --avatar-text), the search highlight and the window close button.
const EXEMPT = /^--(paper(-text|-muted|-rule)?|sender-\d|highlight|window-close|on-window-close)$/;
{
  const { light, dark } = readThemes();
  const missing = [...light]
    .filter(([name, value]) => (value.startsWith('#') || name === '--shadow') && !EXEMPT.test(name) && !dark.has(name))
    .map(([name]) => name);
  assert.deepEqual(missing, [], `tokens without a dark value: ${missing.join(', ')}`);
  console.log(`ok   every colour token of the light block has a dark value (${dark.size} declared)`);
}

// ---- helpers that run in the page ------------------------------------------------------------------------------
/** Installs window.__paint: the colours really painted for an element, and its text contrast. */
const installPaint = () => {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  const rgba = (css) => {
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = '#000';
    ctx.fillStyle = css;
    ctx.fillRect(0, 0, 1, 1);
    const d = ctx.getImageData(0, 0, 1, 1).data;
    return [d[0], d[1], d[2], d[3] / 255];
  };
  const over = (top, under) => [0, 1, 2].map((i) => top[i] * top[3] + under[i] * (1 - top[3])).concat([1]);
  const lum = ([r, g, b]) => {
    const f = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  // The element's background laid over a base colour, from the page down. Two bases (white, black): if the
  // results differ, something translucent lets the canvas of the page show through.
  const background = (el, base) => {
    const chain = [];
    for (let node = el; node; node = node.parentElement) chain.unshift(rgba(getComputedStyle(node).backgroundColor));
    return chain.reduce((acc, layer) => over(layer, acc), base);
  };
  window.__paint = (el) => {
    let opacity = 1;
    for (let node = el; node; node = node.parentElement) opacity *= parseFloat(getComputedStyle(node).opacity);
    const onWhite = background(el, [255, 255, 255, 1]);
    const onBlack = background(el, [0, 0, 0, 1]);
    const seeThrough = [0, 1, 2].some((i) => Math.abs(onWhite[i] - onBlack[i]) > 2);
    const color = rgba(getComputedStyle(el).color);
    color[3] *= opacity;
    const ink = over(color, onBlack);
    const [hi, lo] = [lum(ink), lum(onBlack)].sort((a, b) => b - a);
    return {
      ratio: (hi + 0.05) / (lo + 0.05),
      seeThrough,
      background: onBlack.slice(0, 3).map(Math.round),
      ink: ink.slice(0, 3).map(Math.round),
    };
  };
};

/** Audits every visible text in the page: contrast >= 4.5, never white on white; returns the offenders. */
const auditText = () => {
  const out = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  const seen = new Set();
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (!n.nodeValue.trim()) continue;
    const el = n.parentElement;
    if (!el || seen.has(el) || ['SCRIPT', 'STYLE'].includes(el.tagName)) continue;
    seen.add(el);
    const style = getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    if (style.display === 'none' || style.visibility === 'hidden' || !rect.width || !rect.height) continue;
    if (el.closest('[disabled], [data-disabled], [aria-disabled="true"], [inert]')) continue;
    const p = window.__paint(el);
    const white = p.ink.every((v) => v > 235) && p.background.every((v) => v > 235);
    if (p.ratio < 4.5 || white || p.seeThrough) {
      out.push(`${el.tagName.toLowerCase()}.${el.className || ''} "${n.nodeValue.trim().slice(0, 24)}" ${p.ratio.toFixed(2)}:1 ink=${p.ink} bg=${p.background}${white ? ' WHITE-ON-WHITE' : ''}${p.seeThrough ? ' TRANSPARENT' : ''}`);
    }
  }
  return out;
};

/** The surfaces the primitives draw: each must be opaque (nothing of the page shows through) and not white. */
const auditSurfaces = (selectors) => {
  const out = [];
  for (const selector of selectors) {
    const found = [...document.querySelectorAll(selector)].filter((el) => el.getBoundingClientRect().width);
    if (!found.length) {
      out.push(`${selector}: not on the page`);
      continue;
    }
    for (const el of found) {
      const p = window.__paint(el);
      const style = getComputedStyle(el).backgroundColor;
      if (p.seeThrough) out.push(`${selector}: transparent (${style})`);
      if (p.background.every((v) => v > 235)) out.push(`${selector}: white background (${p.background})`);
    }
  }
  return out;
};

/** A colour token as the browser resolves it, "rgb(r, g, b)". */
const tokenRgb = (page, name) =>
  page.evaluate((token) => {
    const probe = document.createElement('i');
    probe.style.color = `var(--${token})`;
    document.body.append(probe);
    const value = getComputedStyle(probe).color;
    probe.remove();
    return value;
  }, name);
const accentRgb = (page) => tokenRgb(page, 'accent');

/** The resting border of a field is --border-strong (all four sides the same, 1px). */
async function assertFieldBorder(page, field, what) {
  const strong = await tokenRgb(page, 'border-strong');
  const border = await field.evaluate((el) => {
    const s = getComputedStyle(el);
    return { color: s.borderTopColor, width: s.borderTopWidth };
  });
  assert.equal(border.color, strong, `${what}: resting border is --border-strong`);
  assert.equal(border.width, '1px', `${what}: resting border width`);
  console.log(`ok   resting border of ${what} is --border-strong`);
}

/** The focus ring of an element: 2px, solid, in the accent. `ring` is the element that carries it. */
async function assertRing(page, ring, what, accent) {
  const style = await ring.evaluate((el) => {
    const s = getComputedStyle(el);
    return { width: s.outlineWidth, style: s.outlineStyle, color: s.outlineColor };
  });
  assert.equal(style.width, '2px', `${what}: focus ring width`);
  assert.equal(style.style, 'solid', `${what}: focus ring style`);
  assert.equal(style.color, accent, `${what}: focus ring is in --accent`);
}

/** Focuses with the keyboard modality so :focus-visible applies, and checks the ring. */
async function focusAndCheck(page, target, ring, what, accent) {
  await target.focus();
  await page.keyboard.press('Shift');
  await assertRing(page, ring, what, accent);
  console.log(`ok   focus ring of ${what}`);
}

async function audit(page, what, surfaces) {
  await page.evaluate(() => Promise.all(document.getAnimations().map((a) => a.finished.catch(() => {}))));
  await page.waitForTimeout(150);
  const problems = [...(await page.evaluate(auditText)), ...(surfaces ? await page.evaluate(auditSurfaces, surfaces) : [])];
  assert.deepEqual(problems, [], `${what}: ${problems.join('\n  ')}`);
  console.log(`ok   ${what}: text at 4.5:1, no transparent or white surface`);
}

const failures = [];
async function section(name, run) {
  try {
    await run();
  } catch (error) {
    failures.push(`${name}: ${error.message}`);
    console.error(`FAIL ${name}\n  ${error.message}`);
  }
}

// ---- 2. the real app -----------------------------------------------------------------------------------------
{
  const { browser, page, errors } = await openApp({ theme: 'dark' });
  await page.evaluate(installPaint);
  await section('app', async () => {
    assert.equal(await page.evaluate(() => document.documentElement.dataset.scheme), 'dark');
    const luma = (rgb) => {
      const [r, g, b] = rgb.match(/\d+(\.\d+)?/g).map(Number);
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    const sidebar = await page.locator('.sidebar').first().evaluate((el) => getComputedStyle(el).backgroundColor);
    const surface = await page.evaluate(() => {
      const probe = document.createElement('i');
      probe.style.backgroundColor = 'var(--surface)';
      document.body.append(probe);
      const value = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return value;
    });
    assert.ok(luma(sidebar) < luma(surface), `sidebar ${sidebar} is not darker than --surface ${surface}`);
    console.log(`ok   sidebar ${sidebar} is darker than --surface ${surface}`);

    await page.locator('.mail-row').first().locator('.mail-row-content').click();
    await page.locator('.message-body').first().waitFor();
    await page.waitForTimeout(500);
    const body = await page.locator('.message-body').first().evaluate((el) => window.__paint(el));
    assert.ok(body.ratio >= 4.5, `message body ${body.ratio.toFixed(2)}:1 (ink ${body.ink}, bg ${body.background})`);
    assert.ok(!body.seeThrough, 'the message body background is transparent');
    console.log(`ok   message body reads at ${body.ratio.toFixed(2)}:1`);

    // The row subject and the sidebar items read too.
    for (const [selector, what] of [['.mail-row .mail-row-content', 'mail row'], ['.sidebar button', 'sidebar button']]) {
      const p = await page.locator(selector).first().evaluate((el) => window.__paint(el));
      assert.ok(p.ratio >= 4.5, `${what} ${p.ratio.toFixed(2)}:1`);
      console.log(`ok   ${what} reads at ${p.ratio.toFixed(2)}:1`);
    }
    assert.deepEqual(errors, [], `page errors: ${errors.join('; ')}`);
  });
  await browser.close();
}

// ---- 3. the primitives, through the gallery, in the dark ---------------------------------------------------------
{
  const { browser, page, errors } = await openApp({ theme: 'dark' });
  const open = async (which, props) => {
    await mountGallery(page, which, props);
    await page.evaluate(() => (document.documentElement.dataset.scheme = 'dark'));
    await page.evaluate(installPaint);
    return accentRgb(page);
  };

  await section('button', async () => {
    const accent = await open('button');
    const root = page.locator('section[aria-label="Button"]');
    const tipped = root.getByRole('button', { name: 'Archive' });
    await focusAndCheck(page, tipped, tipped, 'icon button', accent);
    await page.waitForTimeout(800);
    const send = root.getByRole('button', { name: 'Send' });
    await focusAndCheck(page, send, send, 'primary button', accent);
    const cancel = root.getByRole('button', { name: 'Cancel' });
    await focusAndCheck(page, cancel, cancel, 'secondary button', accent);
    // The tip (a dark chip) is on the page while the icon button has the focus.
    await tipped.focus();
    await page.keyboard.press('Shift');
    await audit(page, 'button', ['.primary', '.secondary']);
  });

  await section('select', async () => {
    const accent = await open('select');
    const trigger = page.locator('[data-scope="select"][data-part="trigger"]').first();
    await assertFieldBorder(page, trigger, 'select trigger');
    await focusAndCheck(page, trigger, trigger, 'select trigger', accent);
    await trigger.click();
    await page.getByRole('option', { name: 'English' }).waitFor();
    await audit(page, 'select (open)', ['[data-scope="select"][data-part="content"]', '[data-scope="select"][data-part="trigger"]']);
    await page.keyboard.press('Escape');
  });

  await section('checkbox', async () => {
    const accent = await open('checkbox');
    const input = page.getByRole('checkbox', { name: 'Notify me' });
    await input.focus();
    await page.keyboard.press('Shift');
    const control = page.locator('[data-scope="checkbox"][data-part="control"]').first();
    await assertRing(page, control, 'checkbox', accent);
    console.log('ok   focus ring of checkbox');
    await input.press('Space');
    await audit(page, 'checkbox and switch', ['[data-scope="checkbox"][data-part="control"][data-state="checked"]', '[data-scope="switch"][data-part="control"]']);
  });

  await section('switch', async () => {
    const accent = await open('checkbox');
    const input = page.getByRole('switch', { name: 'Dark mode' });
    await input.focus();
    await page.keyboard.press('Shift');
    await assertRing(page, page.locator('[data-scope="switch"][data-part="control"]').first(), 'switch', accent);
    console.log('ok   focus ring of switch');
    await input.press('Space');
    await audit(page, 'switch (on)', ['[data-scope="switch"][data-part="control"][data-state="checked"]', '[data-scope="switch"][data-part="thumb"]']);
  });

  await section('slider', async () => {
    const accent = await open('slider');
    const thumb = page.getByRole('slider', { name: 'Volume' });
    await focusAndCheck(page, thumb, thumb, 'slider thumb', accent);
    await audit(page, 'slider', ['[data-scope="slider"][data-part="thumb"]', '[data-scope="slider"][data-part="track"]', '[data-scope="slider"][data-part="range"]']);
  });

  await section('date', async () => {
    const accent = await open('date');
    const control = page.locator('[data-scope="date-picker"][data-part="control"]').first();
    await assertFieldBorder(page, control, 'date field');
    const input = page.getByRole('textbox', { name: 'Due date' });
    await input.focus();
    await page.keyboard.press('Shift');
    await assertRing(page, control, 'date field', accent);
    console.log('ok   focus ring of date field');
    await page.locator('[data-scope="date-picker"][data-part="trigger"]').first().click();
    await page.locator('[data-scope="date-picker"][data-part="content"]').first().waitFor();
    await audit(page, 'date field (calendar open)', ['[data-scope="date-picker"][data-part="content"]', '[data-scope="date-picker"][data-part="control"]']);
    const day = page.locator('[data-scope="date-picker"][data-part="table-cell-trigger"][tabindex="0"]').first();
    await day.focus();
    await page.keyboard.press('Shift');
    await assertRing(page, day, 'calendar day', accent);
    console.log('ok   focus ring of calendar day');
  });

  await section('native field', async () => {
    await open('button');
    // The base rule of input, textarea and select (system.css), on a bare element.
    await page.evaluate(() => {
      document.body.insertAdjacentHTML('beforeend', '<input id="n-input" aria-label="n"><textarea id="n-area" aria-label="t"></textarea><select id="n-select" aria-label="s"><option>a</option></select>');
    });
    for (const id of ['n-input', 'n-area', 'n-select']) await assertFieldBorder(page, page.locator(`#${id}`), `native #${id}`);
  });

  await section('tabs', async () => {
    const accent = await open('tabs');
    const tab = page.getByRole('tab', { name: /All/ });
    await focusAndCheck(page, tab, tab, 'tab', accent);
    await audit(page, 'tabs (underline)', ['[data-scope="tabs"][data-part="trigger"][data-selected]']);
    await open('tabs', { variant: 'segment' });
    await audit(page, 'tabs (segment)', ['[data-scope="tabs"][data-part="list"]']);
  });

  await section('modal', async () => {
    const accent = await open('modal');
    await page.getByRole('button', { name: 'Open medium' }).click();
    const dialog = page.getByRole('dialog');
    await dialog.waitFor();
    const confirm = dialog.getByRole('button', { name: 'Confirm' });
    await focusAndCheck(page, confirm, confirm, 'modal button', accent);
    await audit(page, 'modal', ['[data-scope="dialog"][data-part="content"]']);
    await page.keyboard.press('Escape');
  });

  await section('side panel', async () => {
    const accent = await open('side-panel');
    await page.getByRole('button', { name: 'Open wide' }).click();
    const panel = page.getByRole('dialog', { name: 'Details' });
    await panel.waitFor();
    const action = panel.getByRole('button', { name: 'Inside action' });
    await focusAndCheck(page, action, action, 'side panel button', accent);
    await audit(page, 'side panel', ['[data-scope="dialog"][data-part="content"][data-variant="side"]']);
  });

  await section('popover', async () => {
    const accent = await open('popover');
    const trigger = page.getByRole('button', { name: 'Options' });
    await focusAndCheck(page, trigger, trigger, 'popover trigger', accent);
    await trigger.click();
    await page.getByRole('button', { name: 'First action' }).waitFor();
    await audit(page, 'popover', ['[data-scope="popover"][data-part="content"]']);
  });

  assert.deepEqual(errors, [], `page errors: ${errors.join('; ')}`);
  await browser.close();
}

if (failures.length) {
  console.error(`\nFAIL ${failures.length} section(s):\n${failures.join('\n')}`);
  process.exit(1);
}
console.log('PASS compatto-dark');
