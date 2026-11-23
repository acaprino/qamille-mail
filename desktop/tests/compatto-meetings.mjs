// The simple Meetings page in the Compatto look (spec 2026-11-09 riunioni-semplici, "Il dettaglio",
// and the ark-compatto spec, sections 6 and 7.2), measured in computed values and driven through
// roles and accessible names, in en-US. The tabs are the Ark Tabs underline variant, the buttons
// and the search field are 32px high with a 6px radius and no resting shadow, the menu and the
// dialogs are Ark, and the dark theme reads at 4.5:1. Synthetic preview data only; no audio device.
// Run: node scripts/test-browser.mjs compatto-meetings
import { expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { openApp, css, box, underline } from './support/compatto.mjs';

const results = [];
const step = async (name, work) => {
  try {
    await work();
  } catch (error) {
    // A step that fails is told, and the next ones still run: one pass shows every difference from Sigrid, not the first.
    process.exitCode = 1;
    console.log(`FAIL STEP ${name}: ${String(error?.message ?? error).split('\n').slice(0, 8).join(' | ').slice(0, 700)}`);
    return;
  }
  results.push(name);
  console.log(`PASS ${name}`);
};

const { browser, page, errors } = await openApp();
const workspace = page.getByRole('navigation', { name: 'Workspace' });
const root = page.locator('.meetings-page');
const pane = page.locator('.meetings-page .detail-pane');
const tab = (name) => pane.getByRole('tab', { name, exact: true });
const rowButton = (title) =>
  root
    .locator('[data-row-key]')
    .filter({ has: page.locator('.item-title', { hasText: new RegExp(`^${title}$`) }) })
    .getByRole('button');
const openMeeting = async (title) => {
  await rowButton(title).click();
  await expect(pane.getByRole('heading', { name: title, level: 2 })).toBeVisible();
};
/** The colour a token paints, as the browser reports it (hex tokens come back as rgb()). */
const token = (name) =>
  page.evaluate((property) => {
    const probe = document.createElement('i');
    probe.style.color = `var(${property})`;
    document.body.append(probe);
    const value = getComputedStyle(probe).color;
    probe.remove();
    return value;
  }, name);
const radius = (locator) => css(locator, 'border-top-left-radius');

try {
  await workspace.getByRole('button', { name: 'Meetings', exact: true }).click();
  await expect(root).toBeVisible();
  await expect(root.locator('[data-list-ready]')).toBeAttached();

  // A meeting that went through the whole chain (transcript, summary, ticked-able action), made in
  // the preview's own pipeline and waited for in real time (about five seconds).
  const recorded = await page.evaluate(async () => {
    const api = await import('/src/lib/meetings.ts');
    const started = await api.meetingCall('meeting_start', {
      request: { request_id: 'compatto-1', title: 'Compact check', microphone: 'demo-mic', system: 'demo-system' },
    });
    await api.meetingCall('meeting_stop');
    for (let i = 0; i < 80; i++) {
      const detail = await api.meetingCall('meeting_read', { id: started.meeting.id });
      if (detail.active_summary) return detail.meeting.id;
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    throw new Error('The preview chain did not finish');
  });
  assert.ok(recorded);
  await page.getByRole('navigation', { name: 'Mail folders' }).getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
  await workspace.getByRole('button', { name: 'Meetings', exact: true }).click();
  await expect(root.locator('[data-list-ready]')).toBeAttached();

  await step('Page chrome: "Record now" and the search field are 32px high, 6px radius, no resting shadow', async () => {
    const record = page.getByRole('button', { name: 'Record now', exact: true });
    const search = page.getByRole('searchbox', { name: 'Search transcripts' });
    for (const [name, control] of [['Record now', record], ['the search field', search]]) {
      assert.equal(Math.round((await box(control)).height), 32, `${name} is 32px high`);
      assert.equal(await radius(control), '6px', `${name} has a 6px radius`);
      assert.equal(await css(control, 'box-shadow'), 'none', `${name} has no resting shadow`);
    }
  });

  await step('Tabs: Sigrid tabs, the chosen one on a tint, arrows, Home and End move', async () => {
    await openMeeting('Weekly sync');
    const tabs = pane.getByRole('tablist', { name: 'Meeting content' });
    await expect(tabs).toBeVisible();
    assert.equal(await tabs.evaluate((node) => node.dataset.slot), 'tabs-list', 'the tablist is Sigrid’s');
    await expect(tabs.getByRole('tab')).toHaveText(['Summary', 'Transcript']);
    const summary = tab('Summary');
    await expect(summary).toHaveAttribute('aria-selected', 'true');
    // The panel is the tab's own: it is named after it.
    await expect(pane.getByRole('tabpanel', { name: 'Summary' })).toBeVisible();

    await summary.focus();
    await page.keyboard.press('ArrowRight');
    await expect(tab('Transcript')).toHaveAttribute('aria-selected', 'true');
    await expect(pane.getByRole('tabpanel', { name: 'Transcript' })).toBeVisible();
    assert.equal(await css(pane.locator('.lines'), 'user-select'), 'text', 'the transcript is reading content: it can be selected');
    await page.keyboard.press('Home');
    await expect(summary).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('End');
    await expect(tab('Transcript')).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('ArrowLeft');
    await expect(summary).toHaveAttribute('aria-selected', 'true');
    // A click works the same.
    await tab('Transcript').click();
    await expect(tab('Transcript')).toHaveAttribute('aria-selected', 'true');
    await tab('Summary').click();
  });

  await step('Detail buttons and checkboxes: 32px controls, Ark checkbox, no native control left', async () => {
    await openMeeting('Compact check');
    const copy = pane.getByRole('button', { name: 'Copy', exact: true });
    const more = pane.getByRole('button', { name: 'More actions', exact: true });
    assert.equal((await box(copy)).height, 32, 'Copy is 32px high');
    assert.equal(await radius(copy), '6px');
    assert.equal(await css(copy, 'box-shadow'), 'none');
    assert.equal((await box(more)).height, 32, 'the menu button is 32px high');
    assert.equal(await radius(more), '6px');
    assert.equal(await more.getAttribute('aria-haspopup'), 'menu');

    // The action is an Ark Checkbox; there is no native select, checkbox or tab list drawn by hand.
    const action = pane.getByRole('checkbox', { name: 'Preparare la revisione del progetto.' });
    assert.equal(await action.evaluate((node) => node.closest('[data-scope="checkbox"]') !== null), true);
    assert.equal(await root.locator('select').count(), 0, 'no native <select>');
    assert.equal(
      await root.locator('input[type="checkbox"]').evaluateAll((nodes) => nodes.filter((node) => !node.closest('[data-scope="checkbox"]')).length),
      0,
      'every checkbox sits in an Ark Checkbox',
    );
    assert.equal(await root.locator('[role="tablist"]').evaluateAll((nodes) => nodes.filter((node) => node.dataset.scope !== 'tabs').length), 0);
    await expect(action).not.toBeChecked();
    await pane.getByText('Preparare la revisione del progetto.', { exact: true }).click();
    await expect(action).toBeChecked();
    await pane.getByText('Preparare la revisione del progetto.', { exact: true }).click();
    await expect(action).not.toBeChecked();
  });

  await step('Menu and dialogs are Ark: a menu, a labelled modal with a 32px field, Esc gives the focus back', async () => {
    const more = pane.getByRole('button', { name: 'More actions', exact: true });
    await more.click();
    const menu = page.getByRole('menu', { name: 'Meeting actions' });
    await expect(menu).toBeVisible();
    assert.equal(await menu.evaluate((node) => node.dataset.slot), 'dropdown-menu-content', 'the menu is Sigrid\u2019s');
    await expect(menu.getByRole('menuitem')).toHaveText(['Rename', 'Delete']);
    // Delete is destructive: last, after a separator.
    assert.deepEqual(await menu.evaluate((node) => [...node.children].map((child) => child.getAttribute('role'))), ['menuitem', 'separator', 'menuitem']);
    await menu.getByRole('menuitem', { name: 'Rename' }).click();

    const dialog = page.getByRole('dialog', { name: 'Rename meeting' });
    await expect(dialog).toBeVisible();
    assert.equal(await dialog.evaluate((node) => node.dataset.slot), 'dialog-content', 'the dialog is Sigrid\u2019s');
    const name = dialog.getByLabel('Name', { exact: true });
    await expect(name).toBeFocused();
    // (Computed, not measured: the dialog is still scaling in.)
    assert.equal(await css(name, 'height'), '32px', 'the name field is 32px high');
    assert.equal(await radius(name), '6px');
    for (const button of ['Save', 'Cancel']) {
      assert.equal(await css(dialog.getByRole('button', { name: button, exact: true }), 'height'), '32px', `${button} is 32px high`);
    }
    // The focus stays inside while it is open.
    for (let i = 0; i < 6; i++) {
      await page.keyboard.press('Tab');
      await expect.poll(() => dialog.evaluate((node) => node.contains(document.activeElement)), { message: 'the focus does not leave the dialog' }).toBe(true);
    }
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(more).toBeFocused();

    // Delete asks in a small Ark dialog; Cancel leaves the meeting where it was.
    await more.click();
    await page.getByRole('menu', { name: 'Meeting actions' }).getByRole('menuitem', { name: 'Delete' }).click();
    const confirm = page.getByRole('dialog', { name: 'Delete meeting' });
    await expect(confirm).toContainText('Delete “Compact check”? The recording and the text are removed. This cannot be undone.');
    await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(confirm).toHaveCount(0);
    await expect(pane.getByRole('heading', { name: 'Compact check', level: 2 })).toBeVisible();
  });

  await step('Dark theme: the list sub-row, the header, the summary, the tabs and the transcript read at 4.5:1', async () => {
    await page.evaluate(() => (document.documentElement.dataset.scheme = 'dark'));
    await page.evaluate(() => {
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 1;
      const ctx = canvas.getContext('2d', { willReadFrequently: true });
      const rgba = (value) => {
        ctx.clearRect(0, 0, 1, 1);
        ctx.fillStyle = '#000';
        ctx.fillStyle = value;
        ctx.fillRect(0, 0, 1, 1);
        const d = ctx.getImageData(0, 0, 1, 1).data;
        return [d[0], d[1], d[2], d[3] / 255];
      };
      const over = (top, under) => [0, 1, 2].map((i) => top[i] * top[3] + under[i] * (1 - top[3])).concat([1]);
      const lum = ([r, g, b]) => {
        const f = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
        return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
      };
      // The element's background laid over a base colour, from the page down: two bases tell a
      // translucent layer from a solid one.
      const paint = (el, base) => {
        const chain = [];
        for (let node = el; node; node = node.parentElement) chain.unshift(rgba(getComputedStyle(node).backgroundColor));
        return chain.reduce((acc, layer) => over(layer, acc), base);
      };
      window.__ratio = (el) => {
        let opacity = 1;
        for (let node = el; node; node = node.parentElement) opacity *= parseFloat(getComputedStyle(node).opacity);
        const onBlack = paint(el, [0, 0, 0, 1]);
        const onWhite = paint(el, [255, 255, 255, 1]);
        const seeThrough = [0, 1, 2].some((i) => Math.abs(onWhite[i] - onBlack[i]) > 2);
        const color = rgba(getComputedStyle(el).color);
        color[3] *= opacity;
        const ink = over(color, onBlack);
        const [hi, lo] = [lum(ink), lum(onBlack)].sort((a, b) => b - a);
        return { ratio: (hi + 0.05) / (lo + 0.05), seeThrough };
      };
    });
    const readable = async (name, locator) => {
      await expect(locator.first()).toBeVisible();
      const { ratio, seeThrough } = await locator.first().evaluate((node) => window.__ratio(node));
      assert.equal(seeThrough, false, `${name} sits on a solid surface`);
      assert.ok(ratio >= 4.5, `${name} reads at ${ratio.toFixed(2)}:1 (needs 4.5:1)`);
    };
    // The list: the title and the sub-row of the open meeting (on the selection colour) and of another.
    await openMeeting('Weekly sync');
    const open = root.locator('[data-row-key]').filter({ has: page.locator('.item-title', { hasText: /^Weekly sync$/ }) });
    await readable('the title of the open row', open.locator('.item-title'));
    await readable('the sub-row of the open row', open.locator('.item-sub'));
    await readable('the sub-row of another row', root.locator('[data-row-key]').filter({ hasNotText: 'Weekly sync' }).locator('.item-sub'));
    // The header, the tabs, the summary.
    await readable('the title', pane.locator('.detail-title'));
    await readable('the date, length and guests', pane.locator('.detail-meta'));
    await readable('the chosen tab', tab('Summary'));
    await readable('the other tab', tab('Transcript'));
    await readable('the summary text', pane.locator('.prose p'));
    await readable('a summary heading', pane.locator('.prose h4'));
    await readable('Copy', pane.getByRole('button', { name: 'Copy', exact: true }));
    // The transcript, with its times (a meeting that has them).
    await openMeeting('Compact check');
    await tab('Transcript').click();
    await readable('the transcript text', pane.locator('.seg span'));
    await readable('the time of a line', pane.locator('.seg time'));
    // The chosen tab is the one that says so.
    await expect(tab('Transcript')).toHaveAttribute('aria-selected', 'true');
  });

  assert.deepEqual(errors, [], `Uncaught browser errors: ${errors.join('; ')}`);
  console.log(`compatto-meetings: ${results.length} steps ok`);
} finally {
  await browser.close();
}
