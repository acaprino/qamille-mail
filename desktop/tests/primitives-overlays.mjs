// Modal, SidePanel and Popover primitives (spec 2026-11-08 ark-compatto §8, Task 4), mounted alone
// through tests/support/mount.mjs: Esc closes through the callback (the veil does not), the focus goes back
// to the opener, the measures come from the --modal-* and --panel-* tokens, the side panel is not
// modal, and a Modal opened from a Popover closes first (Review Focus 2). Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { mountGallery } from './support/mount.mjs';

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const errors = [];
const watch = (page) => page.on('pageerror', (error) => errors.push(error.message));
const count = (page, name) => page.locator(`output[aria-label="${name}"]`).textContent();
// Ark attaches its Esc and outside-press tracking a frame after the surface shows: wait until the focus is
// inside it (which happens in the same step) before sending keys or presses.
const ready = (locator) => expect.poll(() => locator.evaluate((el) => el.contains(document.activeElement))).toBe(true);
const width = (locator) => locator.evaluate((el) => getComputedStyle(el).width);

try {
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1280, height: 800 } });

  // ---------------------------------------------------------------- Modal
  {
    const page = await context.newPage();
    watch(page);
    await mountGallery(page, 'modal');
    const dialog = page.getByRole('dialog', { name: 'Sample dialog' });
    const small = page.getByRole('button', { name: 'Open small', exact: true });

    // (a) Esc calls onclose once, the dialog goes and the focus returns to the button that opened it.
    await small.click();
    await expect(dialog).toBeVisible();
    await ready(dialog);
    assert.equal(await dialog.getAttribute('aria-modal'), 'true');
    // Svelte moved a [data-ds-portal] wrapper to <body>; React's Ark Portal renders the layer there itself.
    assert.equal(await page.locator('body > [data-scope="dialog"][data-part="positioner"]').count(), 1, 'an open Modal is one layer in <body>');
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await page.waitForTimeout(200);
    assert.equal(await count(page, 'Close count'), '1', 'Esc calls onclose exactly once');
    await expect(small).toBeFocused();
    // Closing leaves nothing behind in <body>: the moved wrapper goes with the dialog.
    assert.equal(await page.locator('body > [data-scope="dialog"]').count(), 0, 'a closed Modal leaves a layer in <body>');
    console.log('PASS Modal: Esc calls onclose once and the focus returns to the opener');

    // The widths come from the three --modal-* tokens.
    for (const [opener, expected] of [['Open small', '420px'], ['Open medium', '560px'], ['Open large', '960px']]) {
      await page.getByRole('button', { name: opener, exact: true }).click();
      await expect(dialog).toBeVisible();
      await ready(dialog);
      assert.equal(await width(dialog), expected, `${opener}: width`);
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
    }
    console.log('PASS Modal: sm, md and lg are 420px, 560px and 960px wide');

    // The veil is the --scrim color, and a click on it calls onclose.
    await small.click();
    await expect(dialog).toBeVisible();
    const backdrop = page.locator('[data-scope="dialog"][data-part="backdrop"]');
    const [veil, scrim] = await backdrop.evaluate((el) => {
      const probe = document.createElement('i');
      probe.style.background = 'var(--scrim)';
      document.body.append(probe);
      const expected = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return [getComputedStyle(el).backgroundColor, expected];
    });
    assert.equal(veil, scrim, 'the veil has the color of --scrim');
    const before = Number(await count(page, 'Close count'));
    await page.mouse.click(8, 8);
    await expect(dialog).toBeVisible();
    assert.equal(Number(await count(page, 'Close count')), before, 'a click on the veil does not call onclose (a half-filled form is not lost)');
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    assert.equal(Number(await count(page, 'Close count')), before + 1, 'Esc still calls onclose once');
    await expect(small).toBeFocused();
    console.log('PASS Modal: the veil is --scrim, a click on it does not close, Esc does');

    // Modal: the focus stays inside (Tab wraps), and a button that closes it by state gives the focus back.
    await small.click();
    await expect(dialog).toBeVisible();
    await ready(dialog);
    for (let i = 0; i < 7; i++) {
      await page.keyboard.press('Tab');
      assert.ok(await dialog.evaluate((el) => el.contains(document.activeElement)), `Tab ${i + 1} stays inside the dialog`);
    }
    await dialog.getByRole('button', { name: 'Confirm', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(small).toBeFocused();
    console.log('PASS Modal: Tab stays inside, and closing by a button returns the focus to the opener');

    // The enter animation lasts --motion-panel and is off with prefers-reduced-motion.
    await small.click();
    await expect(dialog).toBeVisible();
    await ready(dialog);
    const motion = await dialog.evaluate((el) => {
      const probe = document.createElement('i');
      probe.style.transitionDuration = 'var(--motion-panel)';
      document.body.append(probe);
      const panel = getComputedStyle(probe).transitionDuration;
      probe.remove();
      return { duration: getComputedStyle(el).animationDuration, panel, name: getComputedStyle(el).animationName };
    });
    assert.notEqual(motion.name, 'none');
    assert.equal(motion.duration, motion.panel, 'the animation lasts --motion-panel');
    await page.keyboard.press('Escape');
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await small.click();
    await expect(dialog).toBeVisible();
    await ready(dialog);
    assert.equal(await dialog.evaluate((el) => getComputedStyle(el).animationName), 'none', 'no animation with prefers-reduced-motion');
    await page.keyboard.press('Escape');
    console.log('PASS Modal: the animation lasts --motion-panel and stops with prefers-reduced-motion');
    await page.close();
  }

  // ---------------------------------------------------------------- SidePanel
  {
    const page = await context.newPage();
    watch(page);
    await mountGallery(page, 'side-panel');
    const panel = page.getByRole('dialog', { name: 'Details' });
    const after = page.getByRole('button', { name: 'After the pane', exact: true });

    await page.getByRole('button', { name: 'Open wide', exact: true }).click();
    await expect(panel).toBeVisible();
    assert.equal(await width(panel), '760px', 'wide panel');
    assert.equal(await panel.getAttribute('aria-modal'), 'false');
    // onopen runs once the panel is showing and its focus choice is kept.
    await expect(page.getByRole('button', { name: 'Close details' })).toBeFocused();
    assert.equal(await count(page, 'Open count'), '1');

    // Not modal: Tab leaves the panel and reaches the rest of the page.
    let reached = false;
    for (let i = 0; i < 6 && !reached; i++) {
      await page.keyboard.press('Tab');
      reached = await after.evaluate((el) => el === document.activeElement);
    }
    assert.ok(reached, 'Tab reaches the button after the pane: the panel does not trap the focus');
    assert.equal(await page.getByRole('button', { name: 'Open wide', exact: true }).getAttribute('aria-hidden'), null);
    await expect(panel).toBeVisible();
    console.log('PASS SidePanel: not modal, the page stays reachable with Tab, onopen places the focus');

    // A click on the scrim calls onscrim only; Esc calls onclose.
    await page.mouse.click(100, 250);
    assert.equal(await count(page, 'Scrim count'), '1', 'the scrim click calls onscrim');
    assert.equal(await count(page, 'Close count'), '0', 'the scrim click does not call onclose');
    await expect(panel).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
    assert.equal(await count(page, 'Close count'), '1', 'Esc calls onclose once');
    console.log('PASS SidePanel: the scrim calls onscrim, Esc calls onclose');

    await page.getByRole('button', { name: 'Open narrow', exact: true }).click();
    await expect(panel).toBeVisible();
    assert.equal(await width(panel), '420px', 'narrow panel');
    console.log('PASS SidePanel: 760px wide, 420px with narrow');

    // Closed with the focus inside it, the panel gives the focus back to the button that opened it; a focus
    // that has moved to the page on its own stays where it is.
    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Open narrow', exact: true })).toBeFocused();
    await page.getByRole('button', { name: 'Open wide', exact: true }).click();
    await expect(panel).toBeVisible();
    await after.focus();
    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
    await expect(after).toBeFocused();
    console.log('PASS SidePanel: the focus returns to the opener on close, unless it had moved to the page');
    await page.close();
  }

  // ---------------------------------------------------------------- Popover
  {
    const page = await context.newPage();
    watch(page);
    await mountGallery(page, 'popover');
    const trigger = page.getByRole('button', { name: 'Options', exact: true });
    const menu = page.getByRole('dialog', { name: 'Row actions' });

    // (c) Opens from its trigger, Esc closes it and the focus returns to the trigger.
    await expect(menu).toHaveCount(0);
    await trigger.click();
    await expect(menu).toBeVisible();
    await ready(menu);
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
    assert.equal(await page.locator('body > [data-scope="popover"]').count(), 0, 'a closed Popover leaves a layer in <body>');
    console.log('PASS Popover: opens from the trigger, Esc closes it and the focus returns to the trigger');

    // An item runs and closes it; a press outside closes it.
    await trigger.click();
    await menu.getByRole('button', { name: 'First action', exact: true }).click();
    await expect(menu).toHaveCount(0);
    assert.equal(await page.locator('output[aria-label="Picked"]').textContent(), 'first');
    assert.equal(await page.locator('body > [data-scope="popover"]').count(), 0, 'a Popover closed by an item leaves a layer in <body>');
    await trigger.click();
    await expect(menu).toBeVisible();
    await ready(menu);
    await page.mouse.click(900, 600);
    await expect(menu).toHaveCount(0);
    console.log('PASS Popover: an item closes it, a press outside closes it');

    // (d) Review Focus 2: a Modal opened from inside the open Popover. The first Esc closes the Modal
    // and leaves the Popover, the second closes the Popover; the focus ends on the trigger.
    const confirm = page.getByRole('dialog', { name: 'Confirm' });
    await trigger.click();
    await expect(menu).toBeVisible();
    await ready(menu);
    const opener = menu.getByRole('button', { name: 'Open dialog', exact: true });
    await opener.click();
    await expect(confirm).toBeVisible();
    await ready(confirm);
    await page.keyboard.press('Escape');
    await expect(confirm).toHaveCount(0);
    await expect(page.locator('[data-scope="popover"][data-part="content"]')).toBeVisible();
    await expect(opener).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();
    console.log('PASS Nested: the first Esc closes the Modal, the second the Popover, the focus ends on the trigger');

    // The same by pointer: the veil closes nothing; Esc closes the Modal only.
    await trigger.click();
    await menu.getByRole('button', { name: 'Open dialog', exact: true }).click();
    await expect(confirm).toBeVisible();
    await ready(confirm);
    await page.mouse.click(8, 8);
    await expect(confirm).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(confirm).toHaveCount(0);
    await expect(page.locator('[data-scope="popover"][data-part="content"]')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    console.log('PASS Nested: a click on the veil leaves the Modal, Esc closes the Modal and leaves the Popover');

    // (e) Z-order, read with a tooltip, a panel, a popover and a modal all in use.
    await expect(trigger).toBeFocused();
    await page.keyboard.press('Shift+Tab');
    await expect(page.getByRole('button', { name: 'Hint', exact: true })).toBeFocused();
    const hint = page.getByRole('tooltip');
    await expect(hint).toBeVisible();
    const layer = (selector) => page.locator(selector).evaluate((el) => Number(getComputedStyle(el).zIndex));
    const tooltipZ = await layer('[data-scope="tooltip"][data-part="positioner"]');
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Open panel', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Side details' })).toBeVisible();
    // The panel (20) is over the popover (10), so the item is reached with the keyboard.
    await trigger.click();
    await menu.getByRole('button', { name: 'Open dialog', exact: true }).focus();
    await page.keyboard.press('Enter');
    await expect(confirm).toBeVisible();
    await ready(confirm);
    const popoverZ = await layer('[data-scope="popover"][data-part="positioner"]');
    const panelZ = await layer('[data-scope="dialog"][data-part="positioner"][data-variant="side"]');
    const modalZ = await layer('[data-scope="dialog"][data-part="positioner"][data-variant="modal"]');
    const token = (name) =>
      page.evaluate((n) => {
        const probe = document.createElement('i');
        probe.style.zIndex = `var(${n})`;
        probe.style.position = 'relative';
        document.body.append(probe);
        const value = Number(getComputedStyle(probe).zIndex);
        probe.remove();
        return value;
      }, name);
    assert.equal(popoverZ, await token('--z-popover'), 'popover is --z-popover');
    assert.equal(panelZ, await token('--z-panel'), 'side panel is --z-panel');
    assert.equal(modalZ, await token('--z-modal'), 'modal is --z-modal');
    assert.equal(tooltipZ, await token('--z-toast'), 'tooltip is --z-toast');
    assert.ok(popoverZ < panelZ && panelZ < modalZ && modalZ < tooltipZ, `popover ${popoverZ} < panel ${panelZ} < modal ${modalZ} < toast ${tooltipZ}`);
    console.log(`PASS Layers: popover ${popoverZ} < panel ${panelZ} < modal ${modalZ} < toast ${tooltipZ}, all from the --z-* tokens`);
    await page.close();
  }

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
