// shortcutIgnored (src/lib/shortcut-guard.ts), the guard of App.tsx's keyboard shortcuts: keys
// typed in a text field, an editable area or an Ark widget that reads keys itself are not the
// page's. It needs a real DOM (Element.closest), so the module is loaded in a browser page from the
// Vite server and run against elements built there. Run by node --test. Synthetic data only.
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
        selectTrigger: '<div data-scope="select" data-part="root"><button id="t">x</button></div>',
        selectItem: '<div data-scope="select" data-part="content"><div><span id="t">English</span></div></div>',
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
        closedModal: at('<div role="dialog" aria-modal="true" data-state="closed" hidden></div><button id="t">x</button>'),
        nonModalOutside: at('<div role="dialog" aria-modal="false" data-state="open"></div><button id="t">x</button>'),
      };
    });
    for (const name of ['sidePanel', 'popover', 'menu', 'openModal']) assert.equal(result[name], true, `${name} must cover the page`);
    for (const name of ['plain', 'closedModal', 'nonModalOutside']) assert.equal(result[name], false, `${name} must not cover the page`);
  } finally {
    await browser.close();
  }
});
