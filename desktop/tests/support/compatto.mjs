// Shared helpers for the Compatto screen tests (one test file per screen, so parallel branches do
// not collide): open the preview with sample data in en-US, and read computed styles.
import { chromium } from '@playwright/test';

export const previewUrl = process.env.QAMILLE_PREVIEW_URL ?? 'http://127.0.0.1:1420';

// Returns { browser, context, page, errors }. `theme` is 'light' or 'dark'; `size` is { width, height }.
export async function openApp({ theme = 'light', size = { width: 1440, height: 900 } } = {}) {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const context = await browser.newContext({ locale: 'en-US', viewport: size, colorScheme: theme });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(previewUrl);
  await page.locator('.compose-main').waitFor();
  if (theme === 'dark') await page.evaluate(() => (document.documentElement.dataset.scheme = 'dark'));
  return { browser, context, page, errors };
}

// Computed style of the first element a locator matches, e.g. css(page.locator('aside'), 'background-color').
export const css = (locator, property) =>
  locator.first().evaluate((node, name) => getComputedStyle(node).getPropertyValue(name), property);

// Bounding box height/width in CSS pixels.
export const box = async (locator) => {
  const found = await locator.first().boundingBox();
  if (!found) throw new Error('element has no box');
  return found;
};

// A token of theme.css as the browser computes it for `property` (default: color), e.g.
// tokenAs(page, '--radius-md', 'border-top-left-radius') -> '8px'. The screen tests compare against
// the token, not a literal, so a change of the look (ADR-0011 v3: unlumen) moves them together.
export const tokenAs = (page, name, property = 'color') =>
  page.evaluate(
    ([token, prop]) => {
      const probe = document.createElement('i');
      probe.style.setProperty(prop, `var(${token})`);
      document.body.append(probe);
      const resolved = getComputedStyle(probe).getPropertyValue(prop);
      probe.remove();
      return resolved;
    },
    [name, property],
  );

// A control at rest carries at most the soft --shadow-sm of the unlumen look (a primary button adds its
// inset top highlight after it), never the floating --elevation-lg. Under Compatto --shadow-sm was `none`.
export async function restingShadow(page, locator) {
  const shadow = await css(locator, 'box-shadow');
  const soft = await tokenAs(page, '--shadow-sm', 'box-shadow');
  return shadow === 'none' || shadow === soft || shadow.startsWith(`${soft}, `) ? 'resting' : shadow;
}

// The underline of a tab in the `underline` Tabs: unlumen's sliding highlight (data-part="highlight",
// ui/Tabs.tsx) has replaced Compatto's 2px border-bottom. { height, color } of the bar
// under the tab, or null when the tab has none.
export const underline = (tab) =>
  tab.first().evaluate((node) => {
    const bar = node.querySelector('[data-part="highlight"]');
    if (!bar) return null;
    const style = getComputedStyle(bar);
    return { height: style.height, color: style.backgroundColor };
  });
