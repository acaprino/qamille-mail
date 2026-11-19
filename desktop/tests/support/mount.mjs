// Mounts one section of tests/support/Gallery.tsx in an otherwise empty page that loads the
// app's /src/styles.css, so a primitive can be tested without the rest of the app (the pattern of
// calendar-flow.mjs and mcp-settings.mjs, kept here so every primitive test uses the same one).
//
//   import { mountGallery } from './support/mount.mjs';
//   await mountGallery(page, 'button');
//   await mountGallery(page, 'select', { items: ['A', 'B'] });
//
// The page is a stub answered by page.route; the module it loads, tests/support/gallery-entry.js,
// goes through the Vite server like any app module, so React, Ark and the primitives resolve to
// the very copies the app uses. The address is QAMILLE_PREVIEW_URL (npm run test:browser sets it).
//
// A fixture that mounts one app component instead (a stub page of its own) imports
// /tests/support/react-mount.js, which has the shape of Svelte's old `mount(Component, { target, props })`.
export async function mountGallery(page, which, props = {}) {
  const base = process.env.QAMILLE_PREVIEW_URL ?? 'http://127.0.0.1:1420';
  // "<" is escaped so no value can close the script element; it stays valid JSON.
  const config = JSON.stringify({ which, props }).replaceAll('<', '\\u003c');
  await page.route('**/__gallery', (route) =>
    route.fulfill({
      contentType: 'text/html',
      body: `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/src/styles.css"></head><body><main id="gallery"></main><script type="application/json" id="gallery-config">${config}</script><script type="module" src="/tests/support/gallery-entry.js"></script></body></html>`,
    }),
  );
  await page.goto(`${base}/__gallery`);
  await page.waitForFunction(() => window.__galleryReady === true);
}
