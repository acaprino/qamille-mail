// Modal, SidePanel and Popover primitives (spec 2026-11-08 ark-compatto §8, Task 4), mounted alone
// through tests/support/mount.mjs: Esc closes through the callback (the veil does not), the focus goes back
// to the opener, the measures come from the --modal-* and --panel-* tokens, the side panel is not
// modal, and a Modal opened from a Popover closes first (Review Focus 2). Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { mountGallery } from './support/mount.mjs';

const base = process.env.QAMILLE_PREVIEW_URL ?? 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
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
    // Sigrid's Dialog renders its layer (a portal) on <body> itself.
    assert.equal(await page.locator('body > [data-slot="dialog-portal"]').count(), 1, 'an open Modal is one layer in <body>');
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
    const backdrop = page.locator('[data-slot="dialog-overlay"]');
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
    await expect(page.locator('[data-slot="popover-content"]')).toBeVisible();
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
    await expect(page.locator('[data-slot="popover-content"]')).toBeVisible();
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

  // ---------------------------------------------------------------- Menu: the reader's "More actions"
  // A command menu is a real menu (Task 12, audit Q16 F15): role menu with menu items, the arrows, Home
  // and End move, a letter jumps to the item that starts with it, Esc gives the focus back to the trigger.
  {
    const page = await context.newPage();
    watch(page);
    await page.goto(base);
    await expect(page.locator('.message-title')).toBeVisible();
    const subject = await page.locator('.message-title').textContent();
    const trigger = page.getByRole('button', { name: 'More message actions', exact: true });
    const menu = page.getByRole('menu', { name: 'Message actions' });
    // The item the keys are on: the only one marked, and the one the menu names as its active descendant.
    const on = async (name) => {
      const marked = menu.locator('[data-highlighted]');
      await expect(marked).toHaveCount(1);
      await expect(marked).toHaveAccessibleName(name);
      await expect(menu).toHaveAttribute('aria-activedescendant', await marked.getAttribute('id'));
    };

    // (a) role menu, items role menuitem; the trigger says it opens a menu.
    await trigger.click();
    await expect(menu).toBeVisible();
    await ready(menu);
    await expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
    await expect(page.getByRole('dialog', { name: 'Message actions' })).toHaveCount(0);
    await expect(menu.getByRole('menuitem')).toHaveText(['Print email', 'Print thread', 'Download .eml']);
    // Set aside is a switch: a menu item that says whether it is on.
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Set aside' })).toHaveAttribute('aria-checked', 'false');
    assert.equal(await menu.locator('button').count(), 0, 'no plain button left inside the menu');
    console.log('PASS Menu (reader): role menu with menu items, and the trigger has aria-haspopup="menu"');

    // (b) Down moves from item to item, Up back; Home and End go to the first and the last.
    await page.keyboard.press('ArrowDown');
    await on('Print email');
    await page.keyboard.press('ArrowDown');
    await on('Print thread');
    await page.keyboard.press('ArrowDown');
    await on('Download .eml');
    await page.keyboard.press('ArrowUp');
    await on('Print thread');
    await page.keyboard.press('End');
    await on('Set aside');
    await page.keyboard.press('Home');
    await on('Print email');
    console.log('PASS Menu (reader): Down and Up move between the items, Home and End go to the ends');

    // (c) A letter brings the item that starts with it; the same letter again, the next one. The letter is
    // the menu's: E does not archive the message behind it.
    await page.keyboard.press('d');
    await on('Download .eml');
    await page.waitForTimeout(500); // the typeahead forgets the letters typed after 350 ms
    await page.keyboard.press('s');
    await on('Set aside');
    await page.waitForTimeout(500);
    await page.keyboard.press('p');
    await on('Print email');
    await page.keyboard.press('p');
    await on('Print thread');
    await page.waitForTimeout(500);
    await page.keyboard.press('e');
    await page.waitForTimeout(300);
    await expect(menu).toBeVisible();
    await expect(page.locator('.message-title')).toHaveText(subject);
    console.log('PASS Menu (reader): a letter jumps to the item that starts with it, and never reaches the shortcuts');

    // (d) Esc closes it and gives the focus back to the trigger; nothing stays in <body>.
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
    assert.equal(await page.locator('body > [data-scope="menu"]').count(), 0, 'a closed Menu leaves a layer in <body>');
    console.log('PASS Menu (reader): Esc closes it and the focus returns to the trigger');

    // From the keyboard: Enter on the trigger opens on the first item. Down on the trigger opens the menu
    // too, and does not step to the next message (on a menu button the arrows are the menu's).
    const position = page.locator('.reader-position');
    const where = await position.textContent();
    // Ark moves the focus into the menu on the next frame: a key pressed before then goes to the trigger,
    // so each step waits for it.
    await page.keyboard.press('Enter');
    await expect(menu).toBeVisible();
    await ready(menu);
    await on('Print email');
    await page.keyboard.press('Escape');
    await expect(trigger).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(menu).toBeVisible();
    await ready(menu);
    await on('Print email');
    await page.waitForTimeout(300);
    await expect(position).toHaveText(where);
    await expect(page.locator('.message-title')).toHaveText(subject);
    console.log('PASS Menu (reader): Enter or Down on the trigger opens it on the first item, and Down stays on the message');

    // A choice runs the action and gives the focus back to the trigger: Set aside, then back.
    await page.keyboard.press('End');
    await on('Set aside');
    await page.keyboard.press('Enter');
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await trigger.click();
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Set aside' })).toHaveAttribute('aria-checked', 'true');
    await menu.getByRole('menuitemcheckbox', { name: 'Set aside' }).click();
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await trigger.click();
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Set aside' })).toHaveAttribute('aria-checked', 'false');
    await page.keyboard.press('Escape');
    console.log('PASS Menu (reader): a choice, by Enter or by click, runs it and returns the focus to the trigger');
    await page.close();
  }

  // ---------------------------------------------------------------- Menu: the primitive alone
  {
    const page = await context.newPage();
    watch(page);
    await mountGallery(page, 'menu');
    const trigger = page.getByRole('button', { name: 'More row actions', exact: true });
    const menu = page.getByRole('menu', { name: 'Row actions' });
    const picked = page.locator('output[aria-label="Picked"]');
    const items = menu.locator('[role^="menuitem"]');
    const on = async (name) => {
      const marked = menu.locator('[data-highlighted]');
      await expect(marked).toHaveCount(1);
      await expect(marked).toHaveAccessibleName(name);
      await expect(menu).toHaveAttribute('aria-activedescendant', await marked.getAttribute('id'));
    };
    const open = async () => {
      await trigger.click();
      await expect(menu).toBeVisible();
      await ready(menu);
    };

    // The order: the groups as given, a separator between two groups, and the destructive item, listed
    // first, moved to the end after a separator of its own.
    await open();
    const names = ['Print', 'Download', 'Archive', 'Pin', 'Details', 'Delete'];
    await expect(items).toHaveCount(names.length);
    for (const [index, name] of names.entries()) await expect(items.nth(index)).toHaveAccessibleName(name);
    const shape = await menu.evaluate((node) => [...node.children].map((child) => child.getAttribute('role')));
    assert.deepEqual(shape, ['menuitem', 'menuitem', 'menuitem', 'separator', 'menuitemcheckbox', 'menuitem', 'separator', 'menuitem']);
    console.log('PASS Menu: a separator between groups, and the destructive item last, after its own separator');

    // A disabled item says so and why, the arrows skip it, and a click on it does nothing.
    const download = menu.getByRole('menuitem', { name: 'Download', exact: true });
    await expect(download).toHaveAttribute('aria-disabled', 'true');
    await expect(download).toHaveAttribute('title', 'Not while printing');
    await page.keyboard.press('ArrowDown');
    await on('Print');
    await page.keyboard.press('ArrowDown');
    await on('Archive');
    await download.click({ force: true });
    await expect(menu).toBeVisible();
    await expect(picked).toHaveText('none');
    console.log('PASS Menu: a disabled item is marked, skipped by the arrows and does nothing on a click');

    // A shortcut is shown at the end of the item and is not part of its name.
    const print = menu.getByRole('menuitem', { name: 'Print', exact: true });
    await expect(print.locator('kbd')).toHaveText('Ctrl+P');
    await expect(print).toHaveAttribute('aria-keyshortcuts', 'Ctrl+P');
    console.log('PASS Menu: a shortcut shows in a <kbd> and is announced as aria-keyshortcuts, not as the name');

    // A click chooses: the action runs once, the menu closes and the focus is back on the trigger.
    await menu.getByRole('menuitem', { name: 'Archive', exact: true }).click();
    await expect(menu).toHaveCount(0);
    await expect(picked).toHaveText('archive');
    await expect(trigger).toBeFocused();
    assert.equal(await page.locator('body > [data-scope="menu"]').count(), 0, 'a Menu closed by a choice leaves a layer in <body>');
    // Enter chooses the item the keys are on: End is the destructive one. (Enter on the trigger opens it;
    // End waits for the focus to be in the menu, which Ark moves there on the next frame.)
    await page.keyboard.press('Enter');
    await expect(menu).toBeVisible();
    await ready(menu);
    await page.keyboard.press('End');
    await on('Delete');
    await page.keyboard.press('Enter');
    await expect(menu).toHaveCount(0);
    await expect(picked).toHaveText('archive,delete');
    await expect(trigger).toBeFocused();
    console.log('PASS Menu: a click or Enter runs the item once and gives the focus back to the trigger');

    // A switch: menuitemcheckbox, its state in aria-checked and a check mark.
    await open();
    const pin = menu.getByRole('menuitemcheckbox', { name: 'Pin', exact: true });
    await expect(pin).toHaveAttribute('aria-checked', 'false');
    assert.equal(await pin.locator('svg').count(), 0, 'an unchecked switch shows no mark');
    await pin.click();
    await expect(menu).toHaveCount(0);
    await open();
    await expect(pin).toHaveAttribute('aria-checked', 'true');
    assert.equal(await pin.locator('svg').count(), 1, 'a checked switch shows its mark');
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();
    console.log('PASS Menu: a switch is a menuitemcheckbox with aria-checked and a check mark');

    // An item that opens a Modal: when the Modal closes, the focus goes back to the menu's trigger.
    await open();
    await menu.getByRole('menuitem', { name: 'Details', exact: true }).click();
    const details = page.getByRole('dialog', { name: 'Details' });
    await expect(details).toBeVisible();
    await ready(details);
    await expect(menu).toHaveCount(0);
    await page.keyboard.press('Escape');
    await expect(details).toHaveCount(0);
    await expect(trigger).toBeFocused();
    console.log('PASS Menu: a Modal opened from an item gives the focus back to the trigger when it closes');

    // Tab does not leave an open menu (Ark): it closes with Esc, a choice or a press outside.
    await open();
    await page.keyboard.press('Tab');
    await expect(menu).toBeVisible();
    await ready(menu);
    await page.keyboard.press('Escape');
    await expect(trigger).toBeFocused();

    // A press outside closes it: on another control the focus stays where the press put it, on the bare
    // page it goes back to the trigger.
    const elsewhere = page.getByRole('button', { name: 'Elsewhere', exact: true });
    await open();
    await elsewhere.click();
    await expect(menu).toHaveCount(0);
    await page.waitForTimeout(200);
    await expect(elsewhere).toBeFocused();
    await open();
    await page.mouse.click(900, 40);
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();
    console.log('PASS Menu: a press outside closes it, and leaves the focus on the control it pressed');

    // The layer is --z-popover, as the Popover's.
    await open();
    const z = await page.locator('[data-scope="menu"][data-part="positioner"]').evaluate((el) => {
      const probe = document.createElement('i');
      probe.style.zIndex = 'var(--z-popover)';
      probe.style.position = 'relative';
      document.body.append(probe);
      const expected = Number(getComputedStyle(probe).zIndex);
      probe.remove();
      return [Number(getComputedStyle(el).zIndex), expected];
    });
    assert.equal(z[0], z[1], 'the menu is on --z-popover');
    await page.keyboard.press('Escape');
    console.log('PASS Menu: on --z-popover');

    // The same look as the Popover the menus used: the same items in both, measured in computed values,
    // at rest, under the pointer and from the keyboard.
    const look = (surface, keyboard) =>
      surface.evaluate((node, keyboard) => {
        const pick = (el, names) => {
          const css = getComputedStyle(el);
          return Object.fromEntries(names.map((n) => [n, css.getPropertyValue(n)]));
        };
        const px = (value) => Math.round(value * 100) / 100;
        const rows = [...node.children].map((row) => {
          const label = [...row.childNodes].find((n) => n.nodeType === 3 && n.textContent.trim()) ?? row.querySelector('span')?.firstChild;
          const range = document.createRange();
          range.selectNodeContents(label);
          const box = row.getBoundingClientRect();
          const icon = row.querySelector('svg').getBoundingClientRect();
          return {
            width: px(box.width),
            height: px(box.height),
            icon: [icon.x - box.x, icon.y - box.y, icon.width, icon.height].map(px),
            label: px(range.getBoundingClientRect().x - box.x),
            style: pick(row, [
              'display', 'align-items', 'gap', 'padding-top', 'padding-right', 'padding-bottom', 'padding-left', 'border-top-left-radius',
              'font-family', 'font-size', 'font-weight', 'line-height', 'color', 'background-color', 'opacity', 'white-space', 'cursor',
              ...(keyboard ? ['outline-style', 'outline-width', 'outline-color', 'outline-offset'] : []),
            ]),
          };
        });
        const box = node.getBoundingClientRect();
        return {
          width: px(box.width),
          height: px(box.height),
          style: pick(node, [
            'min-width', 'padding-top', 'padding-right', 'padding-bottom', 'padding-left', 'color', 'background-color',
            'border-top-width', 'border-top-style', 'border-top-color', 'border-top-left-radius', 'box-shadow',
            'animation-duration', 'animation-timing-function', 'z-index',
          ]),
          rows,
        };
      }, keyboard);
    const popoverTrigger = page.getByRole('button', { name: 'Open the popover', exact: true });
    const menuTrigger = page.getByRole('button', { name: 'Open the menu', exact: true });
    const popoverLook = page.getByRole('dialog', { name: 'Popover look' });
    const menuLook = page.getByRole('menu', { name: 'Menu look' });
    const measure = async (opener, surface, how) => {
      await page.mouse.move(1, 1);
      if (how === 'keyboard') {
        await opener.focus();
        await page.keyboard.press('Shift+Tab');
        await page.keyboard.press('Tab');
        await page.keyboard.press('Enter');
      } else await opener.click();
      await expect(surface).toBeVisible();
      await ready(surface);
      // Measured once the entry animation (it moves the surface) is over.
      await surface.evaluate((el) => Promise.all(el.getAnimations().map((animation) => animation.finished)));
      if (how === 'pointer') {
        await surface.locator('button, [role^="menuitem"]').first().hover();
        await page.waitForTimeout(100);
      }
      const seen = await look(surface, how === 'keyboard');
      await page.keyboard.press('Escape');
      await expect(surface).toHaveCount(0);
      return seen;
    };
    for (const how of ['rest', 'pointer', 'keyboard']) {
      const before = await measure(popoverTrigger, popoverLook, how);
      const after = await measure(menuTrigger, menuLook, how);
      assert.deepEqual(after, before, `${how}: the Menu looks like the Popover it replaces`);
    }
    console.log('PASS Menu: the same sizes, tokens and states as the Popover, at rest, under the pointer and from the keyboard');
    await page.close();
  }

  // The Sigrid plan's insertion points (docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md): a task writes its
  // new cases under its own comment, and changes the existing cases of its primitive where they are. The blank
  // lines stay, so two tasks never touch neighbouring lines.

  // Task 3: words
  {
    // Sigrid's own words follow the interface language through Qamille's root (src/ui/SigridRoot.tsx).
    const page = await context.newPage();
    watch(page);
    await mountGallery(page, 'words');
    await expect(page.locator('[aria-label="Notices"]')).toHaveCount(1);
    await expect(page.locator('[aria-label="Notifiche"]')).toHaveCount(0);
    await page.getByRole('button', { name: 'Italiano', exact: true }).click();
    await expect(page.locator('[aria-label="Notifiche"]')).toHaveCount(1);
    await expect(page.locator('[aria-label="Notices"]')).toHaveCount(0);
    // The error notice announces itself in the language too.
    await expect(page.locator('[aria-label="Notifiche"]')).toContainText('Errore');
    await page.getByRole('button', { name: 'English', exact: true }).click();
    await expect(page.locator('[aria-label="Notices"]')).toHaveCount(1);
    await expect(page.locator('[aria-label="Notices"]')).toContainText('Error');
    console.log('PASS Words: the notice region and the error word follow the interface language');
    await page.close();
  }

  // Task 10: toaster

  // Task 11: date field

  // Task 12: select

  // Task 13: popover, menu and tooltip

  // Task 14: modal and side panel

  // Task 15: recipients

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
