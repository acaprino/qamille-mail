// Review Focus 2 of the Sigrid plan: Qamille's rules must not win over Sigrid's classes. The same gallery section
// (tests/support/Gallery.tsx) is mounted twice on one page: in the clean room (clean-room.html: Sigrid's CSS alone)
// and in Qamille's page (mount.mjs: /src/styles.css). The computed styles of every Sigrid part, and of what is
// inside a part, must be the same in the two; the radii are left out until P3 (Task 20).
//
//   import { compareWithCleanRoom } from './support/sigrid-leak.mjs';
//   const differences = await compareWithCleanRoom(page, 'switch', { checked: true });
//   assert.deepEqual(differences, []);
//
// A part is an element with `data-slot` (Sigrid's own parts) or `data-scope` and `data-part` (Ark's). The section
// opens what has to be measured open (a prop of its own): only what is in the page is compared.
// The address is QAMILLE_PREVIEW_URL (npm run test:browser sets it), as in mount.mjs.
import fs from 'node:fs';
import path from 'node:path';
import { mountGallery } from './mount.mjs';

const CLEAN_ROOM = path.join(import.meta.dirname, 'clean-room.html');

/** The stylesheets clean-room.html links, as paths: the only ones the clean room may have. */
export function cleanRoomSheets() {
  return [...fs.readFileSync(CLEAN_ROOM, 'utf8').matchAll(/<link\s[^>]*rel="stylesheet"[^>]*href="([^"]+)"/g)].map((m) => m[1]);
}

/** Mounts one section of the gallery in the clean room. Same arguments as mountGallery. */
export async function mountCleanRoom(page, which, props = {}) {
  const base = process.env.QAMILLE_PREVIEW_URL ?? 'http://127.0.0.1:1420';
  // "<" is escaped so no value can close the script element; it stays valid JSON.
  const config = JSON.stringify({ which, props }).replaceAll('<', '\\u003c');
  const html = fs.readFileSync(CLEAN_ROOM, 'utf8').replace('__GALLERY_CONFIG__', () => config);
  await page.route('**/__clean-room', (route) => route.fulfill({ contentType: 'text/html', body: html }));
  await page.goto(`${base}/__clean-room`);
  await page.waitForFunction(() => window.__galleryReady === true);
}

// What is compared: what makes a part look different. Inherited text properties count (a part takes them from
// the page), the cursor and text selection do not (Qamille's native feel sets them on purpose: docs/aspetto-nativo.md).
const RADII = ['border-top-left-radius', 'border-top-right-radius', 'border-bottom-right-radius', 'border-bottom-left-radius'];
export const PROPERTIES = [
  'display', 'position', 'box-sizing', 'width', 'height', 'min-height',
  'margin-top', 'margin-right', 'margin-bottom', 'margin-left',
  'padding-top', 'padding-right', 'padding-bottom', 'padding-left',
  'border-top-width', 'border-right-width', 'border-bottom-width', 'border-left-width',
  'border-top-style', 'border-right-style', 'border-bottom-style', 'border-left-style',
  'border-top-color', 'border-right-color', 'border-bottom-color', 'border-left-color',
  ...RADII,
  'color', 'background-color', 'background-image', 'box-shadow', 'opacity',
  'outline-style', 'outline-width', 'outline-color', 'outline-offset',
  'font-family', 'font-size', 'font-weight', 'line-height', 'letter-spacing', 'text-transform', 'text-decoration-line',
  'white-space', 'gap', 'align-items', 'justify-content', 'flex-direction', 'z-index', 'transform',
];
// Until P3 Qamille's radii (4/6/8/12) are its own tokens and Sigrid's are 4/6/10/10: Task 20 turns this on.
const COMPARE_RADII = false;

/** Runs in the page: every part, and every element inside a part, in document order, with the properties asked. */
function snapshot(properties) {
  const PART = '[data-slot], [data-scope][data-part]';
  const nameOf = (el) => {
    const slot = el.getAttribute('data-slot');
    return slot ? `[data-slot=${slot}]` : `[${el.getAttribute('data-scope')}:${el.getAttribute('data-part')}]`;
  };
  const counts = new Map();
  const rows = [];
  for (const el of document.body.querySelectorAll('*')) {
    const owner = el.closest(PART);
    if (!owner) continue;
    const name = el === owner ? nameOf(el) : `${nameOf(owner)} ${el.tagName.toLowerCase()}`;
    const n = counts.get(name) ?? 0;
    counts.set(name, n + 1);
    const css = getComputedStyle(el);
    const style = Object.fromEntries(properties.map((p) => [p, css.getPropertyValue(p)]));
    // What cannot be seen does not count: the style and colour of a border of no width (CSS computes the width of a
    // border with no style as 0), the colour and offset of an outline with no style.
    for (const side of ['top', 'right', 'bottom', 'left']) {
      if (style[`border-${side}-width`] === '0px') style[`border-${side}-style`] = style[`border-${side}-color`] = '(no border)';
    }
    if (style['outline-style'] === 'none') style['outline-color'] = style['outline-offset'] = '(no outline)';
    rows.push({ key: `${name}#${n}`, style });
  }
  return rows;
}

/**
 * The differences between two snapshots, one line each: a part that only one page has, or a property whose value
 * differs ("<part>: <property> is <in Qamille's page> in Qamille's page, <in the clean room> in the clean room").
 */
export function diffParts(clean, app, { radii = COMPARE_RADII } = {}) {
  const out = [];
  const appByKey = new Map(app.map((row) => [row.key, row]));
  const cleanKeys = new Set(clean.map((row) => row.key));
  for (const row of clean) {
    const other = appByKey.get(row.key);
    if (!other) {
      out.push(`${row.key}: only in the clean room`);
      continue;
    }
    for (const [property, value] of Object.entries(row.style)) {
      if (!radii && RADII.includes(property)) continue;
      if (other.style[property] !== value) out.push(`${row.key}: ${property} is ${other.style[property]} in Qamille's page, ${value} in the clean room`);
    }
  }
  for (const row of app) if (!cleanKeys.has(row.key)) out.push(`${row.key}: only in Qamille's page`);
  return out;
}

/** Mounts `which` in the clean room and then in Qamille's page, and gives the differences of their Sigrid parts. */
export async function compareWithCleanRoom(page, which, props = {}) {
  const take = async () => {
    await page.evaluate(() => document.fonts.ready);
    return page.evaluate(snapshot, PROPERTIES);
  };
  await mountCleanRoom(page, which, props);
  const clean = await take();
  await mountGallery(page, which, props);
  const app = await take();
  return diffParts(clean, app);
}
