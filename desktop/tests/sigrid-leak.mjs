// Review Focus 2 of the Sigrid plan (docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md): Qamille's rules do not
// win over Sigrid's classes. Every primitive moved onto Sigrid is mounted twice by compareWithCleanRoom
// (tests/support/sigrid-leak.mjs): in the clean room, with Sigrid's CSS alone, and in Qamille's page. The computed
// styles of each of its parts must be the same in both, the radii aside until P3. One case per primitive, written
// by its task under its own comment in CASES; until then only the harness is checked. Synthetic data only.
//   npm run test:browser -- sigrid-leak
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import { cleanRoomSheets, compareWithCleanRoom, diffParts, mountCleanRoom } from './support/sigrid-leak.mjs';
import { mountGallery } from './support/mount.mjs';

// { name, which, props }: `which` is the gallery section (tests/support/Gallery.tsx) that shows the primitive with
// what has to be measured open. The blank lines stay, so two tasks never touch neighbouring lines.
const CASES = [
  // Task 4: switch

  // Task 5: tabs

  // Task 6: count and slider

  // Task 7: button

  // Task 8: checkbox

  // Task 9: field

  // Task 10: toaster

  // Task 11: date field

  // Task 12: select

  // Task 13: popover, menu and tooltip

  // Task 14: modal and side panel

  // Task 15: recipients

  // Task 17: nav list

  // Task 19: icons

];

// The comparison itself, on made-up snapshots: equal parts give nothing; a property apart, or a part that only one
// page has, gives one line naming it; the radii are left out until P3.
{
  const part = (key, style) => ({ key, style });
  const base = { color: 'rgb(0, 0, 0)', 'border-top-left-radius': '10px' };
  assert.deepEqual(diffParts([part('[data-slot=button]#0', base)], [part('[data-slot=button]#0', { ...base })]), []);
  assert.deepEqual(diffParts([part('[data-slot=button]#0', base)], [part('[data-slot=button]#0', { ...base, color: 'rgb(1, 1, 1)' })]), [
    "[data-slot=button]#0: color is rgb(1, 1, 1) in Qamille's page, rgb(0, 0, 0) in the clean room",
  ]);
  assert.deepEqual(diffParts([part('[data-slot=button]#0', base)], [part('[data-slot=button]#0', { ...base, 'border-top-left-radius': '8px' })]), []);
  assert.deepEqual(diffParts([part('[data-slot=button]#0', base)], [part('[data-slot=button]#0', { ...base, 'border-top-left-radius': '8px' })], { radii: true }), [
    "[data-slot=button]#0: border-top-left-radius is 8px in Qamille's page, 10px in the clean room",
  ]);
  assert.deepEqual(diffParts([part('[data-slot=a]#0', base)], [part('[data-slot=b]#0', base)]), ['[data-slot=a]#0: only in the clean room', "[data-slot=b]#0: only in Qamille's page"]);
  console.log('PASS diffParts names each property apart and each part that only one page has, radii aside');
}

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const errors = [];
const failures = [];
try {
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  const sheets = () => page.evaluate(() => [...document.styleSheets].map((sheet) => (sheet.href ? new URL(sheet.href).pathname : null)).filter(Boolean));
  const token = (name) => page.evaluate((name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim(), name);

  // The two rooms. The clean one has exactly the sheets clean-room.html links, none of Qamille's base, and none of
  // the tokens only Qamille writes (--z-toast is in theme/qamille-tokens.css); Qamille's has /src/styles.css and them.
  await mountCleanRoom(page, 'none');
  assert.deepEqual(await sheets(), cleanRoomSheets(), 'the clean room has only the sheets clean-room.html links');
  assert.ok(!cleanRoomSheets().some((sheet) => ['/src/styles.css', '/src/theme/base.css', '/src/theme/qamille-tokens.css', '/src/ui/parts.css'].includes(sheet)), 'clean-room.html links none of Qamille\'s own sheets');
  assert.equal(await token('--z-toast'), '', 'no Qamille-only token in the clean room');
  assert.notEqual(await token('--color-paper'), '', 'Sigrid\'s tokens are in the clean room');
  await mountGallery(page, 'none');
  assert.ok((await sheets()).includes('/src/styles.css'), 'Qamille\'s page has /src/styles.css');
  assert.notEqual(await token('--z-toast'), '', 'Qamille\'s tokens are in its page');
  console.log('PASS the clean room has Sigrid\'s CSS alone, and Qamille\'s page has its own');

  for (const { name, which, props } of CASES) {
    const differences = await compareWithCleanRoom(page, which, props);
    if (differences.length) failures.push(`${name}: ${differences.length} differences\n    ${differences.join('\n    ')}`);
    else console.log(`PASS ${name} looks the same in Qamille's page as in the clean room`);
  }
  if (!CASES.length) console.log('sigrid-leak: no primitive on Sigrid yet (each primitive task adds its case)');
  assert.deepEqual(errors, [], `page errors: ${errors.join(' | ')}`);
} finally {
  await browser.close();
}
if (failures.length) {
  console.log(`FAIL Qamille's rules change Sigrid's parts:\n  ${failures.join('\n  ')}`);
  process.exit(1);
}
console.log('sigrid-leak: ok');
