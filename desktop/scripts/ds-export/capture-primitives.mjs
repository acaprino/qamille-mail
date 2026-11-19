// Previews of the primitives, produced by mounting the module's own components (ADR-0011) in the
// harness page, not by looking for their classes in the app.
//
// The Ark parts that exist only while open (the popover, the tooltip, the lists, the calendar, the notices) are
// opened the way a person opens them, by the card's `open` step, and taken from <body>, where the primitives
// move them. A part carries its data-scope and data-part, which ark.css selects on, so its static copy looks
// the same without the machine behind it.
import { capture, freezeClock, newContext, settle } from './browser.mjs';

const states = ['default', 'hover', 'focus', 'disabled'];
const variants = ['primary', 'secondary', 'icon', 'text'];
const cases = (...names) => names.map((n) => `[data-case="${n}"]`);
const part = (scope, name) => `[data-scope="${scope}"][data-part="${name}"]`;
const modal = (name) => `${part('dialog', 'content')}[data-case="${name}"]`;
// A dialog or panel of the harness, mounted by name and removed again.
const overlay = (name, sel, prefix) => ({
  // A panel is taken with its stage, which is always there: wait for the panel inside it.
  open: async (page) => { await page.evaluate((n) => window.__dsOverlay(n), name); await wait(page, sel.includes('data-scope') ? sel : `${sel} ${part('dialog', 'content')}`); },
  sel: [sel],
  prefix,
  close: async (page) => { await page.evaluate(() => window.__dsOverlay('')); await gone(page, part('dialog', 'content')); },
});
const panel = (name, prefix) => overlay(name, `[data-case="${name}"]`, prefix);

const wait = (page, sel, state = 'visible') => page.locator(sel).filter({ visible: true }).first().waitFor({ state, timeout: 15000 });
const gone = (page, sel) => page.locator(sel).first().waitFor({ state: 'hidden', timeout: 15000 });
const away = (page) => page.mouse.move(1439, 1);

// The harness page lists every case; each card takes its own. A card is one or more shots: a shot may open
// something first (`open`), takes its selectors, and puts the page back (`close`).
export const PRIMITIVE_CARDS = [
  { comp: 'Button', shots: [{ sel: variants.flatMap((v) => cases(...states.map((s) => `button-${v}-${s}`))) }] },
  { comp: 'Field', shots: [{ sel: cases('field-text', 'field-focus', 'field-disabled', 'field-help', 'field-textarea') }] },
  { comp: 'Icon', shots: [{ sel: cases('icons') }] },
  { comp: 'Modal', shots: [overlay('modal-sm', modal('modal-sm'), 'a'), overlay('modal-md', modal('modal-md'), 'b'), overlay('modal-lg', modal('modal-lg'), 'c')] },
  { comp: 'SidePanel', shots: [panel('side-panel-wide', 'a'), panel('side-panel-narrow', 'b')] },
  {
    comp: 'Popover',
    shots: [{
      open: async (page) => { await page.locator('[data-case="popover"] button').click(); await wait(page, part('popover', 'content')); },
      sel: ['[data-case="popover"]', part('popover', 'content')],
      close: async (page) => { await page.keyboard.press('Escape'); await gone(page, part('popover', 'content')); },
    }],
  },
  {
    comp: 'Tooltip',
    shots: [{
      // 400 ms of hover open it; the clock is only frozen for Date, so the delay runs.
      open: async (page) => { await page.locator('[data-case="tooltip"] button').hover(); await wait(page, part('tooltip', 'content')); },
      sel: ['[data-case="tooltip"]', part('tooltip', 'content')],
      close: async (page) => { await away(page); await gone(page, part('tooltip', 'content')); },
    }],
  },
  {
    comp: 'Select',
    shots: [{
      open: async (page) => { await page.locator(`[data-case="select-open"] ${part('select', 'trigger')}`).click(); await wait(page, part('select', 'content')); },
      sel: [...cases('select-closed', 'select-open', 'select-empty', 'select-disabled'), part('select', 'content')],
      close: async (page) => { await page.keyboard.press('Escape'); await gone(page, part('select', 'content')); },
    }],
  },
  { comp: 'Checkbox', shots: [{ sel: cases('checkbox-off', 'checkbox-on', 'checkbox-mixed', 'checkbox-disabled') }] },
  { comp: 'Switch', shots: [{ sel: cases('switch-on', 'switch-off', 'switch-disabled') }] },
  { comp: 'Tabs', shots: [{ sel: cases('tabs-underline', 'tabs-segment') }] },
  { comp: 'Slider', shots: [{ sel: cases('slider') }] },
  {
    comp: 'DateField',
    shots: [{
      open: async (page) => { await page.locator(`[data-case="datefield-open"] ${part('date-picker', 'trigger')}`).click(); await wait(page, part('date-picker', 'content')); },
      sel: [...cases('datefield-closed', 'datefield-open'), part('date-picker', 'content')],
      close: async (page) => { await page.keyboard.press('Escape'); await gone(page, part('date-picker', 'content')); },
    }],
  },
  {
    comp: 'Toaster',
    // One notice at a time, so the two kinds are two shots; their ids are told apart by a prefix.
    shots: [
      {
        open: async (page) => { await page.evaluate(() => window.__dsNotify('Messaggio archiviato', false)); await wait(page, `${part('toast', 'root')}[data-type="success"]`); },
        sel: [`${part('toast', 'root')}[data-type="success"]`],
        prefix: 'a',
      },
      {
        open: async (page) => { await page.evaluate(() => window.__dsNotify('Non è stato possibile inviare il messaggio.', true)); await wait(page, `${part('toast', 'root')}[data-type="error"]`); },
        sel: [`${part('toast', 'root')}[data-type="error"]`],
        prefix: 'b',
        close: async (page) => { await page.locator(`${part('toast', 'root')} ${part('toast', 'close-trigger')}`).first().click(); await gone(page, part('toast', 'root')); await away(page); },
      },
    ],
  },
  {
    // Last: the box keeps the text that is typed in it as a chip when the focus leaves.
    comp: 'RecipientsInput',
    shots: [{
      open: async (page) => {
        await page.locator('[data-case="recipients"] input').pressSequentially('mar', { delay: 40 });
        await wait(page, part('combobox', 'content'));
      },
      sel: ['[data-case="recipients"]', part('combobox', 'content')],
    }],
  },
];

/** `icons`: [name, path] from Icon.svelte. Returns the captured cards and the problems found. */
export async function capturePrimitives({ browser, base, want, icons, log }) {
  const frags = [];
  const failures = [];
  // Reduced motion: the side panel's entrance would otherwise be caught half way.
  const context = await newContext(browser, { reducedMotion: 'reduce' });
  try {
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e.message).split('\n')[0]));
    await freezeClock(page);
    await page.addInitScript((names) => { window.__DS_ICONS__ = names; }, icons.map(([name]) => name));
    await page.goto(`${base}/scripts/ds-export/harness/index.html`);
    try {
      await page.waitForSelector('[data-case="icons"] svg', { timeout: 60000 });
    } catch (e) {
      throw new Error(`the harness page did not mount${errors.length ? ` (page errors: ${errors.slice(0, 3).join(' | ')})` : ''}: ${String(e.message).split('\n')[0]}`);
    }
    await settle(page);

    // The mounted component and the table it is built from have to agree: the SVG files of the export come from the table.
    const mounted = await page.evaluate(() => [...document.querySelectorAll('[data-case="icons"] figure')].map((f) => [f.querySelector('figcaption').textContent, f.querySelector('path').getAttribute('d')]));
    const table = new Map(icons);
    for (const [name, d] of mounted) if (table.get(name) !== d) failures.push(`Icon: the mounted "${name}" does not draw the path of the table`);
    if (mounted.length !== icons.length) failures.push(`Icon: ${mounted.length} icons mounted, ${icons.length} in the table`);

    for (const card of PRIMITIVE_CARDS) {
      if (!want(card.comp)) continue;
      try {
        const parts = [];
        const html = [];
        const missing = [];
        for (const shot of card.shots) {
          try {
            await shot.open?.(page);
            const r = await capture(page, shot.sel, { prefix: shot.prefix ?? '' });
            parts.push(...r.parts);
            html.push(r.html);
            missing.push(...r.missing);
          } finally {
            await shot.close?.(page);
            await away(page);
          }
        }
        const r = { parts, html: html.join(''), missing };
        frags.push({ comp: card.comp, group: 'Primitive', source: 'module', ...r });
        log(`  ${card.comp}: ${r.parts.map((p) => `${p.w}x${p.h}`).join(' ')}${r.missing.length ? `, MISSING ${r.missing.join(',')}` : ''}`);
        if (r.missing.length) failures.push(`${card.comp}: not found in the harness: ${r.missing.join(', ')}`);
      } catch (e) {
        failures.push(`${card.comp}: ${String(e.message).split('\n')[0]}`);
      }
    }
    if (errors.length) failures.push(`the harness page raised errors: ${[...new Set(errors)].slice(0, 3).join(' | ')}`);
  } finally {
    await context.close();
  }
  frags.sort((a, b) => PRIMITIVE_CARDS.findIndex((c) => c.comp === a.comp) - PRIMITIVE_CARDS.findIndex((c) => c.comp === b.comp));
  return { frags, failures };
}
