// The sidebar and the shell in the Compatto look (spec 2026-11-08 ark-compatto §6, Task 7): measures
// of the mockup read as computed values at 1440x900 in en-US, and behaviour through role and name.
// Synthetic data only (the preview's sample workspace).
import { expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { openApp, css, box } from './support/compatto.mjs';

const WHITE = 'rgb(255, 255, 255)';
const ACCENT = 'rgb(79, 70, 229)';
const SIDEBAR_ACTIVE = 'rgb(40, 43, 49)';

const channels = (color) => color.match(/[\d.]+/g).slice(0, 3).map(Number);
const luminance = (color) => {
  const [r, g, b] = channels(color).map((value) => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

const { browser, page, errors } = await openApp();
try {
  const sidebar = page.getByRole('complementary', { name: 'Sidebar' });
  const folders = sidebar.getByRole('navigation', { name: 'Mail folders' });
  const accounts = sidebar.getByRole('navigation', { name: 'Accounts' });
  const tools = sidebar.getByRole('navigation', { name: 'Workspace' });

  // (a) The column is the Compatto dark ground.
  assert.equal(await css(sidebar, 'background-color'), 'rgb(21, 23, 27)');
  console.log('PASS The sidebar has the dark ground rgb(21, 23, 27)');

  // (b) New message: accent fill, 36 px high, 6 px radius, the N shown.
  const compose = sidebar.getByRole('button', { name: 'New message' });
  assert.equal(await css(compose, 'background-color'), ACCENT);
  assert.equal((await box(compose)).height, 36);
  assert.equal(await css(compose, 'border-top-left-radius'), '6px');
  await expect(compose.locator('kbd')).toBeVisible();
  await expect(compose.locator('kbd')).toHaveText('N');
  console.log('PASS New message is accent, 36 px, 6 px radius and shows N');

  // (c) Every folder and account row is 32 px high.
  for (const group of [accounts, folders]) {
    const rows = group.getByRole('button');
    const count = await rows.count();
    assert.ok(count >= 3, 'the group has rows');
    for (let i = 0; i < count; i++) assert.equal((await box(rows.nth(i))).height, 32, `row ${i} is 32 px`);
  }
  console.log('PASS Folder and account rows are 32 px high');

  // (d) The active folder: lifted fill, white text, count on the accent. The others: soft white.
  const inbox = folders.getByRole('button', { name: /^Inbox/ });
  await expect(inbox).toHaveAttribute('aria-current', 'page');
  assert.equal(await css(inbox, 'background-color'), SIDEBAR_ACTIVE);
  assert.equal(await css(inbox, 'color'), WHITE);
  const count = inbox.locator('.nav-count');
  await expect(count).toBeVisible();
  assert.equal(await css(count, 'background-color'), ACCENT);
  assert.equal(await css(count, 'color'), WHITE);
  const otherCount = await folders.locator('button:not([aria-current])').count();
  assert.ok(otherCount >= 5);
  for (let i = 0; i < otherCount; i++)
    assert.equal(await css(folders.locator('button:not([aria-current])').nth(i), 'color'), 'rgb(209, 212, 218)');
  console.log('PASS The active folder is lifted with an accent count; the others are rgb(209, 212, 218)');

  // (e) The tools sit at the bottom, above a hairline in --color-sidebar-selected, below the folders.
  const calendar = tools.getByRole('button', { name: 'Calendar' });
  for (const name of ['Calendar', 'Contacts', 'Meetings', 'Assistant'])
    await expect(tools.getByRole('button', { name: new RegExp(`^${name}`) })).toBeVisible();
  assert.equal(await css(tools, 'border-top-width'), '1px');
  assert.equal(await css(tools, 'border-top-color'), SIDEBAR_ACTIVE);
  const folderBox = await box(folders);
  const toolsBox = await box(tools);
  const sidebarBox = await box(sidebar);
  assert.ok(toolsBox.y >= folderBox.y + folderBox.height, 'the tools are below the folders');
  assert.ok(sidebarBox.y + sidebarBox.height - (toolsBox.y + toolsBox.height) < 100, 'the tools are in the bottom of the column');
  assert.equal((await box(calendar)).height, 32);
  console.log('PASS The tools are in the bottom of the column, above a --color-sidebar-selected hairline');

  // (f) Accounts: Sigrid's avatars in the colour of each account, with a legible initial.
  const avatars = sidebar.locator('.accounts-nav [data-slot="avatar"]');
  const avatarCount = await avatars.count();
  assert.ok(avatarCount >= 3);
  for (let i = 0; i < avatarCount; i++) {
    const avatar = avatars.nth(i);
    const [fill, ink] = [await css(avatar, 'background-color'), await css(avatar, 'color')];
    assert.ok(contrast(fill, ink) >= 4.5, `account avatar ${i} text is legible (${fill} on ${ink})`);
  }
  const user = sidebar.locator('[data-slot="avatar"]').last();
  const [userFill, userInk] = [await css(user, 'background-color'), await css(user, 'color')];
  assert.ok(contrast(userFill, userInk) >= 4.5, 'the user initial is legible');
  console.log('PASS Account and user avatars are Sigrid’s, with legible initials');

  // (g) Collapse sidebar: a hint, not a title; the name stays; the hint opens on focus.
  const collapse = sidebar.getByRole('button', { name: 'Collapse sidebar' });
  assert.equal(await collapse.getAttribute('title'), null);
  // Only the account rows keep a title (the address, which the row does not show).
  const titled = await sidebar.locator('[title]').evaluateAll((nodes) => nodes.map((n) => !!n.closest('.accounts-nav')));
  assert.ok(titled.every(Boolean), 'no title attribute is left in the sidebar outside the account rows');
  await sidebar.getByRole('button', { name: 'New message' }).focus();
  await page.keyboard.press('Shift+Tab');
  await expect(collapse).toBeFocused();
  await expect(page.getByRole('tooltip')).toContainText('Collapse sidebar');
  await page.keyboard.press('Escape');
  console.log('PASS Collapse sidebar has a tooltip and no title attribute');

  // (h) Collapsed: the unread dot uses the lightened accent; every button keeps its accessible name.
  await collapse.click();
  const collapsed = page.getByRole('complementary', { name: 'Sidebar' });
  await expect(collapsed.locator('.sidebar-text').first()).toBeHidden();
  const dot = collapsed.locator('.nav-count').first();
  assert.equal(await css(dot, 'background-color'), 'rgb(173, 184, 255)');
  const buttons = collapsed.getByRole('button');
  const total = await buttons.count();
  assert.ok(total >= 14);
  for (let i = 0; i < total; i++) {
    const name = await buttons.nth(i).evaluate((node) => node.getAttribute('aria-label') || node.textContent.trim());
    assert.ok(name, `collapsed button ${i} has a name`);
    assert.ok(await buttons.nth(i).getAttribute('aria-label'), `collapsed button ${i} keeps its aria-label`);
    assert.equal(await buttons.nth(i).getAttribute('title'), null, `collapsed button ${i} has no title`);
  }
  await collapsed.getByRole('button', { name: 'Expand sidebar' }).click();
  console.log('PASS Collapsed: the dot is the lightened accent and every button keeps its aria-label');

  // (i) Narrow windows. The app's smallest window is 1000 px (tauri.conf.json minWidth), so there the
  // reader toolbar must keep every button inside its pane. Below that the page still must not scroll
  // sideways (the toolbar itself is wider than the pane at 850 px, as it was before this task).
  for (const width of [1000, 1100, 1200, 850, 800]) {
    await page.setViewportSize({ width, height: 900 });
    await page.locator('.mail-row').first().click();
    await page.locator('.reader-toolbar').waitFor();
    const overflow = await page.evaluate(() => {
      const toolbar = document.querySelector('.reader-toolbar');
      const edge = toolbar.getBoundingClientRect().right;
      const widest = Math.max(...[...toolbar.querySelectorAll('button')].map((b) => b.getBoundingClientRect().right));
      const side = document.querySelector('.sidebar').getBoundingClientRect();
      return {
        page: document.documentElement.scrollWidth - window.innerWidth,
        toolbar: toolbar.scrollWidth - toolbar.clientWidth,
        button: widest - edge,
        sidebarContent: document.querySelector('.sidebar-body').scrollWidth - document.querySelector('.sidebar-body').clientWidth,
        sidebarWidth: side.width,
      };
    });
    assert.ok(overflow.page <= 0, `the page overflows by ${overflow.page}px at ${width}px`);
    assert.ok(overflow.sidebarContent <= 0, `the sidebar content overflows by ${overflow.sidebarContent}px at ${width}px`);
    if (width >= 1000) {
      assert.ok(overflow.toolbar <= 0, `the reader toolbar scrolls by ${overflow.toolbar}px at ${width}px`);
      assert.ok(overflow.button <= 0, `a toolbar button leaves the toolbar by ${overflow.button}px at ${width}px`);
    }
  }
  console.log('PASS Nothing overflows: toolbar at 1000-1200 px, page and sidebar down to 800 px');

  // (j) The search sits at the centre of the pane to the right of the sidebar; the actions stay right.
  const geometry = () =>
    page.evaluate(() => {
      const r = (selector) => document.querySelector(selector).getBoundingClientRect();
      const top = r('.topbar');
      const search = r('.global-search');
      const actions = r('.topbar-end');
      return {
        centre: Math.abs(search.left + search.width / 2 - (top.left + top.width / 2)),
        width: search.width,
        gapToActions: actions.left - search.right,
        actionsRight: top.right - actions.right,
        top: top.height,
      };
    });
  await page.setViewportSize({ width: 1440, height: 900 });
  const wide = await geometry();
  assert.ok(wide.centre <= 4, `the search is ${wide.centre}px off the centre of the topbar at 1440 px`);
  assert.ok(wide.width <= 560 && wide.width >= 540, `the search is ${wide.width}px wide at 1440 px`);
  assert.ok(wide.actionsRight <= 14, 'the actions stay on the right');
  assert.equal(wide.top, 48);
  assert.equal(await css(page.locator('.global-search'), 'border-top-color'), await page.evaluate(() => {
    const probe = document.createElement('i');
    probe.style.color = 'var(--color-input)';
    document.body.append(probe);
    const value = getComputedStyle(probe).color;
    probe.remove();
    return value;
  }));
  await page.setViewportSize({ width: 1000, height: 700 });
  const narrow = await geometry();
  assert.ok(narrow.gapToActions >= 0, `the search overlaps the actions by ${-narrow.gapToActions}px at 1000 px`);
  assert.ok(narrow.width >= 240, `the search is ${narrow.width}px wide at 1000 px`);
  console.log('PASS The search is centred in the topbar at 1440 px and clear of the actions at 1000 px');

  assert.deepEqual(errors, []);
  console.log('Compatto sidebar checks passed');
} finally {
  await browser.close();
}
