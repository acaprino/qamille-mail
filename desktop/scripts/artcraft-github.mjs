// The latest GitHub release of acaprino/artcraft-web, unpacked like artcraft-web's dist/: <cache>/<tag>/<engine>/.
// The download itself (latest release of main, manifest, SHA-256, cache) is github-release.mjs, shared with the
// Sigrid; this file is what is specific to ArtCraft: the four engines and how each is unpacked.
// Resolves to { tag, dir }, or null (no network, no release): the caller then falls back to a local artcraft-web.
import { rmSync } from 'node:fs';
import { join } from 'node:path';
import { latestMainRelease, untar } from './github-release.mjs';

export const ARTCRAFT_REPO = 'acaprino/artcraft-web';
export const ARTCRAFT_ENGINES = ['pdfcraft', 'wordcraft', 'gridcraft', 'deckcraft'];

// `api` is where GitHub's API is; only the tests replace it.
export function latestArtcraftRelease(cache, log = console.log, api) {
  return latestMainRelease({
    repo: ARTCRAFT_REPO,
    cache,
    label: 'artcraft-web',
    log,
    api,
    // The four engines, in this order; a release without one of them is unusable.
    select: (manifest, tag) => ARTCRAFT_ENGINES.map((engine) => manifest.packages.find((p) => p.engine === engine) ?? (() => { throw new Error(`${engine} is not in the manifest of ${tag}`); })()),
    unpack(file, part, pkg) {
      const into = join(part, pkg.engine);
      untar(file, into);
      // The npm package is dist/<engine> plus these two.
      for (const extra of ['package.json', 'README.md']) rmSync(join(into, extra), { force: true });
    },
  });
}
