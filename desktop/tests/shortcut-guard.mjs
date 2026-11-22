// shortcutIgnored, shortcutCoveredByLayer and arrowsMoveMail (src/lib/shortcut-guard.ts), the guards of
// App.tsx's keyboard shortcuts: keys typed in a text field, an editable area or an Ark widget that
// reads keys itself are not the page's, and the arrows move the open message only from the page, the
// list or the reader. They need a real DOM (Element.closest), so the module is loaded in a browser
// page from the Vite server and run against elements built there; isAppChord is pure and runs here.
// Run by node --test. Synthetic data only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { chromium } from '@playwright/test';

const base = process.env.QAMILLE_PREVIEW_URL;

test('shortcutIgnored: editing and widget targets are ignored, the page is not', { skip: !base && 'QAMILLE_PREVIEW_URL not set' }, async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage();
    await page.route('**/__guard', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><html><body></body></html>' }));
    await page.goto(`${base}/__guard`);
    const result = await page.evaluate(async () => {
      const { shortcutIgnored } = await import('/src/lib/shortcut-guard.ts');
      const cases = {
        selectTrigger: '<button data-slot="select-trigger" id="t">x</button>',
        selectItem: '<div data-slot="select-content" role="listbox"><div role="option"><span id="t">English</span></div></div>',
        comboboxInput: '<div data-scope="combobox"><div id="t">x</div></div>',
        tagsInput: '<div data-scope="tags-input"><span id="t">x</span></div>',
        datePicker: '<div data-scope="date-picker"><button id="t">12</button></div>',
        sliderThumb: '<div data-scope="slider" data-part="control"><div data-part="thumb" role="slider" id="t"></div></div>',
        sliderRole: '<div role="slider" id="t"></div>',
        input: '<input id="t">',
        textarea: '<textarea id="t"></textarea>',
        nativeSelect: '<select id="t"><option>a</option></select>',
        editable: '<div contenteditable="true"><b id="t">x</b></div>',
        editableBare: '<div contenteditable><b id="t">x</b></div>',
        notEditable: '<div contenteditable="false"><b id="t">x</b></div>',
        plain: '<div><button id="t">x</button></div>',
        otherScope: '<div data-scope="tooltip"><span id="t">x</span></div>',
      };
      const out = {};
      for (const [name, html] of Object.entries(cases)) {
        const host = document.createElement('div');
        host.innerHTML = html;
        document.body.append(host);
        out[name] = shortcutIgnored(host.querySelector('#t'));
        host.remove();
      }
      out.body = shortcutIgnored(document.body);
      out.document = shortcutIgnored(document);
      out.window = shortcutIgnored(window);
      out.null = shortcutIgnored(null);
      return out;
    });
    for (const name of ['selectTrigger', 'selectItem', 'comboboxInput', 'tagsInput', 'datePicker', 'sliderThumb', 'sliderRole', 'input', 'textarea', 'nativeSelect', 'editable', 'editableBare']) {
      assert.equal(result[name], true, `${name} must be ignored`);
    }
    for (const name of ['notEditable', 'plain', 'otherScope', 'body', 'document', 'window', 'null']) {
      assert.equal(result[name], false, `${name} must not be ignored`);
    }
  } finally {
    await browser.close();
  }
});

test('shortcutCoveredByLayer: a focused dialog, popover or menu, or any open modal, keeps the keys from the page', { skip: !base && 'QAMILLE_PREVIEW_URL not set' }, async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage();
    await page.route('**/__guard', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><html><body></body></html>' }));
    await page.goto(`${base}/__guard`);
    const result = await page.evaluate(async () => {
      const { shortcutCoveredByLayer } = await import('/src/lib/shortcut-guard.ts');
      const at = (html) => {
        const host = document.createElement('div');
        host.innerHTML = html;
        document.body.append(host);
        const covered = shortcutCoveredByLayer(host.querySelector('#t') ?? document.body);
        host.remove();
        return covered;
      };
      return {
        plain: at('<div class="mail-list"><button id="t">x</button></div>'),
        sidePanel: at('<div role="dialog" aria-modal="false" data-state="open"><button id="t">x</button></div>'),
        popover: at('<div data-scope="popover" role="dialog"><button id="t">x</button></div>'),
        menu: at('<div role="menu"><div role="menuitem" id="t">x</div></div>'),
        // The focus is on the page (e.g. <body>) while a modal confirmation is open over it.
        openModal: at('<div role="dialog" aria-modal="true" data-state="open"></div><button id="t">x</button>'),
        sigridModal: at('<div role="dialog" aria-modal="true" data-open></div><button id="t">x</button>'),
        closedModal: at('<div role="dialog" aria-modal="true" data-state="closed" hidden></div><button id="t">x</button>'),
        nonModalOutside: at('<div role="dialog" aria-modal="false" data-state="open"></div><button id="t">x</button>'),
      };
    });
    for (const name of ['sidePanel', 'popover', 'menu', 'openModal', 'sigridModal']) assert.equal(result[name], true, `${name} must cover the page`);
    for (const name of ['plain', 'closedModal', 'nonModalOutside']) assert.equal(result[name], false, `${name} must not cover the page`);
  } finally {
    await browser.close();
  }
});

test('arrowsMoveMail: only the page, the list and the reader move the open message', { skip: !base && 'QAMILLE_PREVIEW_URL not set' }, async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage();
    await page.route('**/__guard', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><html><body></body></html>' }));
    await page.goto(`${base}/__guard`);
    const result = await page.evaluate(async () => {
      const { arrowsMoveMail } = await import('/src/lib/shortcut-guard.ts');
      const at = (html) => {
        const host = document.createElement('div');
        host.innerHTML = html;
        document.body.append(host);
        const moves = arrowsMoveMail(host.querySelector('#t'));
        host.remove();
        return moves;
      };
      return {
        listRow: at('<div class="mail-list"><div class="mail-row"><button id="t">x</button></div></div>'),
        listItself: at('<div class="mail-list" id="t"></div>'),
        reader: at('<section class="reader"><div class="reader-toolbar"><button id="t">x</button></div></section>'),
        readerFrame: at('<section class="reader"><div class="message-body"><iframe id="t"></iframe></div></section>'),
        // A popover's button in the reader keeps the arrows for the messages; a menu button opens its menu.
        readerPopoverButton: at('<section class="reader"><div class="reader-toolbar"><button id="t" aria-haspopup="dialog">x</button></div></section>'),
        readerMenuButton: at('<section class="reader"><div class="reader-toolbar"><button id="t" aria-haspopup="menu" aria-expanded="false">x</button></div></section>'),
        body: arrowsMoveMail(document.body),
        sidebar: at('<aside class="sidebar"><nav><button id="t">x</button></nav></aside>'),
        sidebarFootRegion: at('<aside class="sidebar"><div role="region" aria-label="Activity"><button id="t">x</button></div></aside>'),
        topbar: at('<header class="topbar"><button id="t">x</button></header>'),
        activityRegion: at('<div role="region" aria-label="Activity centre"><button id="t">x</button></div>'),
        searchBar: at('<div class="search-bar"><button id="t">x</button></div>'),
        plainDiv: at('<div><button id="t">x</button></div>'),
        document: arrowsMoveMail(document),
        window: arrowsMoveMail(window),
        null: arrowsMoveMail(null),
      };
    });
    for (const name of ['listRow', 'listItself', 'reader', 'readerFrame', 'readerPopoverButton', 'body']) assert.equal(result[name], true, `${name} must move the message`);
    for (const name of ['readerMenuButton', 'sidebar', 'sidebarFootRegion', 'topbar', 'activityRegion', 'searchBar', 'plainDiv', 'document', 'window', 'null']) {
      assert.equal(result[name], false, `${name} must not move the message`);
    }
  } finally {
    await browser.close();
  }
});

test('isAppChord: Ctrl or Meta without Alt, so AltGr (Ctrl+Alt on Windows) is not a chord', async () => {
  const { isAppChord } = await import('../src/lib/shortcut-guard.ts');
  assert.equal(isAppChord({ ctrlKey: true, metaKey: false, altKey: false }), true);
  assert.equal(isAppChord({ ctrlKey: false, metaKey: true, altKey: false }), true);
  assert.equal(isAppChord({ ctrlKey: true, metaKey: true, altKey: false }), true);
  // AltGr reports ctrlKey and altKey together: a character typed on an Italian layout, not a command.
  assert.equal(isAppChord({ ctrlKey: true, metaKey: false, altKey: true }), false);
  assert.equal(isAppChord({ ctrlKey: false, metaKey: true, altKey: true }), false);
  assert.equal(isAppChord({ ctrlKey: false, metaKey: false, altKey: true }), false);
  assert.equal(isAppChord({ ctrlKey: false, metaKey: false, altKey: false }), false);
});

// The window-wide keyboard() of App.tsx, in the preview with the sample mailbox: where the arrows
// act, which chords count, and what Esc does in the search field.
test('App keyboard(): arrows only from the page, the list or the reader; AltGr is not a chord; Esc clears the search', { skip: !base && 'QAMILLE_PREVIEW_URL not set' }, async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto(base);
    await page.locator('.mail-row').first().waitFor();
    const selected = () => page.locator('.mail-row.selected .mail-open').first().getAttribute('aria-describedby');
    const openFirst = async () => {
      await page.locator('.mail-row .mail-open').first().click();
      await page.locator('.reader .reader-toolbar').waitFor();
      return selected();
    };
    const search = page.getByRole('textbox', { name: 'Search mail', exact: true });

    // The controls that show a shortcut say so to assistive technology.
    assert.equal(await search.getAttribute('aria-keyshortcuts'), 'Control+K');

    // Down from a top-bar button, the sidebar or the reader's own toolbar: only the reader and the list move.
    const first = await openFirst();
    assert.ok(first, 'the first message is open');
    assert.equal(await page.locator('.reader-reply').getAttribute('aria-keyshortcuts'), 'R');
    assert.equal(await page.getByRole('button', { name: 'Archive message' }).getAttribute('aria-keyshortcuts'), 'E');
    for (const [where, locator] of [
      ['top-bar sync button', page.getByRole('button', { name: 'Sync mail', exact: true })],
      ['sidebar new-message button', page.locator('.compose-main')],
      ['sidebar navigation', page.locator('.sidebar-body button, .sidebar-body a').first()],
    ]) {
      await locator.focus();
      await page.keyboard.press('ArrowDown');
      await page.waitForTimeout(150);
      assert.equal(await selected(), first, `ArrowDown on the ${where} must not open the next message`);
    }
    // From the open message's row in the list it moves on, and from the reader it moves back.
    await page.locator('.mail-row.selected .mail-open').first().focus();
    await page.keyboard.press('ArrowDown');
    await page.waitForFunction((id) => document.querySelector('.mail-row.selected .mail-open')?.getAttribute('aria-describedby') !== id, first);
    const second = await selected();
    assert.notEqual(second, first, 'ArrowDown in the list opens the next message');
    await page.locator('.reader-reply').focus();
    await page.keyboard.press('ArrowUp');
    await page.waitForFunction((id) => document.querySelector('.mail-row.selected .mail-open')?.getAttribute('aria-describedby') !== id, second);
    assert.equal(await selected(), first, 'ArrowUp in the reader opens the previous message');

    // Ctrl+K focuses the search; AltGr (Ctrl+Alt) with the same letter is a character and does not.
    await page.locator('.reader-reply').focus();
    await page.keyboard.press('Control+Alt+k');
    await page.waitForTimeout(150);
    assert.equal(await page.evaluate(() => document.activeElement?.classList.contains('reader-reply')), true, 'Ctrl+Alt+K must leave the focus where it was');
    await page.keyboard.press('Control+k');
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Search mail');

    // Esc empties a search field that has text, leaves an empty one alone, and an IME Escape is not one.
    await search.fill('maria');
    await page.keyboard.press('Escape');
    assert.equal(await search.inputValue(), '', 'Esc empties the search field');
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), 'Search mail', 'and keeps the focus in it');
    await page.keyboard.press('Escape');
    assert.equal(await search.inputValue(), '');
    await search.fill('maria');
    await search.dispatchEvent('keydown', { key: 'Escape', keyCode: 229, isComposing: true, bubbles: true, cancelable: true });
    assert.equal(await search.inputValue(), 'maria', 'the Escape that drops an IME candidate is not a clear');
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});
