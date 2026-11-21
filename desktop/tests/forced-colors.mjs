// Windows Contrast themes (forced colours, audit F10, src/ds/styles/forced-colors.css): against the
// browser preview with `forced-colors: active` emulated, every state that the app shows only with a
// tint or a shadow must still be visible as a border or an outline in a system colour.
// Run: node tests/forced-colors.mjs   (Vite preview on 127.0.0.1:1420). Synthetic sample data only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const shots = process.env.QAMILLE_SHOTS;
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1280, height: 860 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
const results = [];
async function step(name, work) {
  await work();
  results.push(name);
  console.log(`PASS ${name}`);
}

// The used value of a system colour in this palette, read from a probe element.
const systemColor = (keyword) =>
  page.evaluate((keyword) => {
    const probe = document.createElement('span');
    probe.style.color = keyword;
    document.body.append(probe);
    const value = getComputedStyle(probe).color;
    probe.remove();
    return value;
  }, keyword);
const style = (locator, ...properties) =>
  locator.evaluate((element, properties) => {
    const computed = getComputedStyle(element);
    return Object.fromEntries(properties.map((p) => [p, computed.getPropertyValue(p)]));
  }, properties);
const visibleLine = (width, lineStyle) => parseFloat(width) >= 1 && lineStyle !== 'none' && lineStyle !== 'hidden';

try {
  for (const colorScheme of ['dark', 'light']) {
    await page.emulateMedia({ forcedColors: 'active', colorScheme });
    await page.goto(base);
    await page.locator('.mail-row').first().waitFor();
    const highlight = await systemColor('Highlight');

    await step(`${colorScheme}: the page sees a Contrast theme`, async () => {
      assert.equal(await page.evaluate(() => matchMedia('(forced-colors: active)').matches), true);
    });

    await step(`${colorScheme}: the selected message has a highlighted edge, not only a tint`, async () => {
      await page.locator('.mail-row .mail-open').first().click();
      const selected = page.locator('.mail-row.selected').first();
      await expect(selected).toHaveCount(1);
      const s = await style(selected, 'outline-style', 'outline-width', 'outline-color');
      assert.ok(visibleLine(s['outline-width'], s['outline-style']), `selected row outline: ${JSON.stringify(s)}`);
      assert.equal(s['outline-color'], highlight, 'selected row outline is Highlight');
    });

    await step(`${colorScheme}: the active page in the sidebar has a highlighted edge`, async () => {
      const active = page.locator('.sidebar nav button.active').first();
      await expect(active).toHaveCount(1);
      const s = await style(active, 'outline-style', 'outline-width', 'outline-color');
      assert.ok(visibleLine(s['outline-width'], s['outline-style']), `active nav outline: ${JSON.stringify(s)}`);
      assert.equal(s['outline-color'], highlight);
    });

    await step(`${colorScheme}: keyboard focus draws a ring`, async () => {
      await page.locator('.mail-row .mail-open').first().focus();
      await page.keyboard.press('Tab');
      const s = await page.evaluate(() => {
        const element = document.activeElement;
        const computed = getComputedStyle(element);
        return { tag: element.tagName, style: computed.outlineStyle, width: computed.outlineWidth, visible: element.matches(':focus-visible') };
      });
      assert.ok(s.visible && visibleLine(s.width, s.style), `focused ${s.tag}: ${JSON.stringify(s)}`);
    });

    await step(`${colorScheme}: the search field shows its focus (Ctrl+K), drawn on its container`, async () => {
      await page.locator('.mail-row .mail-open').first().focus();
      await page.keyboard.press('Control+k');
      await expect(page.locator('.global-search input')).toBeFocused();
      const s = await style(page.locator('.global-search'), 'outline-style', 'outline-width', 'outline-color');
      assert.ok(visibleLine(s['outline-width'], s['outline-style']), `search outline: ${JSON.stringify(s)}`);
      assert.equal(s['outline-color'], highlight);
      await page.keyboard.press('Escape');
    });

    await step(`${colorScheme}: a priority message keeps its edge`, async () => {
      const priority = page.locator(".mail-row[data-section='priority']").first();
      if (!(await priority.count())) return;
      const s = await style(priority, 'border-left-style', 'border-left-width');
      assert.ok(visibleLine(s['border-left-width'], s['border-left-style']), `priority edge: ${JSON.stringify(s)}`);
    });

    await step(`${colorScheme}: a menu keeps its edge, its separator, and an outline on the current item`, async () => {
      await page.getByRole('button', { name: 'More message actions', exact: true }).click();
      const menu = page.getByRole('menu', { name: 'Message actions' });
      await expect(menu).toBeVisible();
      const edge = await style(menu, 'border-top-style', 'border-top-width');
      assert.ok(visibleLine(edge['border-top-width'], edge['border-top-style']), `menu edge: ${JSON.stringify(edge)}`);
      const line = await style(menu.getByRole('separator').first(), 'background-color');
      assert.equal(line['background-color'], await systemColor('CanvasText'), 'the separator is drawn in CanvasText');
      // The current item, from the keys and then under the pointer: its ring and its tint are gone, the
      // outline stays.
      const current = menu.locator('[data-highlighted]');
      // Ark moves the focus into the menu on the next frame: the arrow waits for it.
      await expect.poll(() => menu.evaluate((el) => el.contains(document.activeElement))).toBe(true);
      await page.keyboard.press('ArrowDown');
      await expect(current).toHaveCount(1);
      for (const how of ['keys', 'pointer']) {
        if (how === 'pointer') {
          await menu.getByRole('menuitem').nth(1).hover();
          await expect(current).toHaveAccessibleName(await menu.getByRole('menuitem').nth(1).textContent());
        }
        const s = await style(current, 'outline-style', 'outline-width', 'outline-color');
        assert.ok(visibleLine(s['outline-width'], s['outline-style']), `${how}: current item outline: ${JSON.stringify(s)}`);
        assert.equal(s['outline-color'], highlight, `${how}: the outline is Highlight`);
      }
      await page.keyboard.press('Escape');
      await expect(menu).toHaveCount(0);
    });

    await step(`${colorScheme}: a switch shows its track and whether it is on`, async () => {
      await page.locator('.sidebar .profile').click();
      await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
      const switches = page.getByRole('switch');
      for (const name of [null, 'Mail', 'Notifications', 'Meetings', 'Search', 'Accounts']) {
        if (await switches.count()) break;
        if (name) await page.locator('.settings-nav').getByRole('button', { name, exact: true }).click().catch(() => {});
      }
      await switches.first().waitFor();
      const states = await switches.evaluateAll((elements) =>
        elements.slice(0, 12).map((element) => {
          const thumb = element.querySelector("[data-part='thumb']");
          const fill = element.querySelector("[data-part='fill']");
          const track = getComputedStyle(element);
          return {
            on: element.getAttribute('aria-checked') === 'true',
            border: track.borderTopStyle !== 'none' && parseFloat(track.borderTopWidth) >= 1,
            thumb: thumb ? getComputedStyle(thumb).backgroundColor : null,
            fill: fill ? getComputedStyle(fill).backgroundColor : null,
            fillOpacity: fill ? Number(getComputedStyle(fill).opacity) : null,
          };
        }),
      );
      assert.ok(states.length > 0, 'Settings has switches');
      const buttonText = await systemColor('ButtonText');
      const highlightText = await systemColor('HighlightText');
      for (const s of states) {
        assert.ok(s.border, `switch track has an edge: ${JSON.stringify(s)}`);
        assert.equal(s.thumb, s.on ? highlightText : buttonText, `switch thumb: ${JSON.stringify(s)}`);
        if (s.on) assert.ok(s.fill === highlight && s.fillOpacity === 1, `an "on" switch is filled with Highlight: ${JSON.stringify(s)}`);
        else assert.equal(s.fillOpacity, 0, `an "off" switch has no fill: ${JSON.stringify(s)}`);
      }
      if (shots) await page.screenshot({ path: `${shots}/forced-colors-settings-${colorScheme}.png` });
    });
  }
  assert.deepEqual(errors, [], `page errors: ${errors.join('\n')}`);
  console.log(`\nforced colors: ${results.length} checks pass`);
} catch (error) {
  console.error(`FAIL ${error.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
