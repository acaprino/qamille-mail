// The browser every capture runs in: same engine, locale, time zone and clock, so that two runs agree.
import { chromium } from '@playwright/test';
import { FROZEN } from './config.mjs';
import { FLATTEN_IFRAMES, serializeFragment } from './serialize.mjs';

export async function launch() {
  try {
    return await chromium.launch({ channel: 'msedge' });
  } catch {
    return await chromium.launch();
  }
}

export function newContext(browser, extra = {}) {
  return browser.newContext({ locale: FROZEN.locale, timezoneId: FROZEN.timezone, viewport: FROZEN.viewport, ...extra });
}

/** The date stands still; timers keep running, so animations and polling still work. */
export const freezeClock = (page) => page.clock.setFixedTime(new Date(FROZEN.time));

/** A page at its starting state: storage cleared, the app loaded with its sample data. */
export async function fresh(page, base) {
  const errors = [];
  const note = (e) => errors.push(String(e.message ?? e).split('\n')[0]);
  page.on('pageerror', note);
  try {
    for (let attempt = 1; ; attempt++) {
      try {
        await page.goto(base);
        await page.evaluate(() => { localStorage.clear(); sessionStorage.clear(); });
        await page.goto(base);
        await page.waitForSelector('button.mail-open', { timeout: 25000 });
        return;
      } catch (e) {
        // The dev server can reload the page when it finds a dependency it had not optimized, and under load
        // that goes on for several seconds ("Navigation is interrupted by another navigation"): wait a little
        // longer each time and try again.
        if (attempt < 6) { await page.waitForTimeout(2000 * attempt).catch(() => {}); continue; }
        if (attempt === 6) throw new Error(`the app did not load after ${attempt} attempts (${String(e.message).split('\n')[0]})${errors.length ? `; page errors: ${errors.slice(-3).join(' | ')}` : ''}`);
      }
    }
  } finally {
    page.off('pageerror', note);
  }
}

// One step: { sel, text, textI (case-insensitive), textEq, has, aria, ariaStart, nth, wait }
export async function step(page, s) {
  const done = await page.evaluate((f) => {
    const label = (e) => e.getAttribute('aria-label') || '';
    let els = [...document.querySelectorAll(f.sel || 'button')];
    if (f.text) els = els.filter((e) => e.innerText.trim().startsWith(f.text));
    if (f.textEq) els = els.filter((e) => e.innerText.trim() === f.textEq);
    // Group headings are set in capitals by CSS, and innerText follows it.
    if (f.textI) els = els.filter((e) => e.innerText.trim().toLowerCase().startsWith(f.textI));
    if (f.has) els = els.filter((e) => e.innerText.includes(f.has));
    if (f.aria) els = els.filter((e) => label(e) === f.aria);
    if (f.ariaStart) els = els.filter((e) => label(e).startsWith(f.ariaStart));
    els = els.filter((e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; });
    const el = els[f.nth || 0];
    if (!el) return false;
    el.click();
    return true;
  }, s);
  if (!done) throw new Error(`step target missing: ${JSON.stringify(s)}`);
  await page.waitForTimeout(s.wait ?? 450);
}

/** Waits until the first thing a card captures is on screen: lazy components arrive after the click that opens them. */
export async function ready(page, selectors) {
  const first = typeof selectors[0] === 'string' ? { sel: selectors[0] } : selectors[0];
  await page.locator(first.sel).filter({ visible: true }).nth(first.nth || 0).waitFor({ timeout: 30000 });
}

/** Lets fonts finish, takes the focus and the pointer off the page, so that no hover or focus ring is captured. */
export async function settle(page, ms = 400) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(ms);
  await page.evaluate(() => document.activeElement?.blur?.());
  await page.mouse.move(1439, 1);
  await page.waitForTimeout(150);
}

/**
 * Serializes until two passes in a row give the same markup: a capture that never settles is an error,
 * not a file that differs from run to run (INV-005).
 */
export async function capture(page, selectors, { attempts = 8, prefix = '' } = {}) {
  let previous = null;
  for (let i = 0; i < attempts; i++) {
    const r = await page.evaluate(serializeFragment, { selectors, flatten: FLATTEN_IFRAMES, prefix });
    if (!r.parts.length) throw new Error(`nothing found: ${r.missing.join(', ')}`);
    const key = JSON.stringify(r);
    if (key === previous) return r;
    previous = key;
    await page.waitForTimeout(200);
  }
  throw new Error('the DOM did not settle');
}
