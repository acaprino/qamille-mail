// Copies the compiled artcraft-web engines into the desktop sources, after checking every file
// against the build's provenance.json. Output is generated and ignored by git.
// Qamille always takes the latest release of artcraft-web's main branch, with no pin of its own:
//   the latest GitHub release of acaprino/artcraft-web, used only when its tag is on main (public, plain HTTPS),
//   cached in desktop/.cache/artcraft-web;
//   ARTCRAFT_WEB_DIR, when set, takes its place: a checkout's dist/ (to try engines that are not released yet);
//   without the network the copy already in src/vendor/artcraft stays.
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ARTCRAFT_ENGINES, latestArtcraftRelease } from './artcraft-github.mjs';

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const explicit = process.env.ARTCRAFT_WEB_DIR;
const release = explicit ? null : await latestArtcraftRelease(join(desktop, '.cache', 'artcraft-web'));
const source = explicit ? resolve(explicit) : null;
const distOf = (engine) => (release ? join(release.dir, engine) : source ? join(source, 'dist', engine) : null);
const engines = process.argv.slice(2).length ? process.argv.slice(2) : ARTCRAFT_ENGINES;
const CODE = (name, engine) => name === `${engine}.js` || name === `${engine}.d.ts` || name === `${engine}_bg.wasm`;
const sha256 = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');

for (const engine of engines) {
  const dist = distOf(engine);
  const code = join(desktop, 'src', 'vendor', 'artcraft', engine);
  const licenses = join(desktop, 'public', 'artcraft-licenses', engine);
  if (!dist || !existsSync(join(dist, 'provenance.json'))) {
    if (existsSync(join(code, `${engine}_bg.wasm`))) {
      console.log(`${engine}: no release of artcraft-web's main reachable, keeping the existing copy`);
      continue;
    }
    throw new Error(`${engine}: ${dist} missing. Check the network (the latest release of acaprino/artcraft-web's main), or build it in artcraft-web (npm run build ${engine}) and set ARTCRAFT_WEB_DIR.`);
  }
  const provenance = JSON.parse(readFileSync(join(dist, 'provenance.json'), 'utf8'));
  // From a checkout, artcraft-web's pins/ are its own build inputs: a bumped pin that has not been built
  // yet is reported, it does not stop the app.
  const pinFile = source && join(source, 'pins', `${engine}.json`);
  if (pinFile && existsSync(pinFile)) {
    const pin = JSON.parse(readFileSync(pinFile, 'utf8'));
    if (pin.commit !== provenance.commit) {
      console.log(`${engine}: artcraft-web pins ${pin.tag} but its latest build is ${provenance.tag}: using ${provenance.tag} (npm run build ${engine} in artcraft-web for ${pin.tag})`);
    }
  }
  const names = readdirSync(dist).filter((name) => name !== 'provenance.json');
  for (const name of names) {
    if (provenance.files?.[name] !== sha256(join(dist, name))) {
      throw new Error(`${engine}: ${name} does not match provenance.json. Rebuild the engine.`);
    }
  }
  for (const name of ['provenance.json', ...names]) if (!existsSync(join(dist, name))) throw new Error(`${engine}: ${name} missing`);
  rmSync(code, { recursive: true, force: true });
  rmSync(licenses, { recursive: true, force: true });
  mkdirSync(code, { recursive: true });
  mkdirSync(licenses, { recursive: true });
  for (const name of names.filter((n) => CODE(n, engine) || n.endsWith('_bg.wasm.d.ts'))) cpSync(join(dist, name), join(code, name));
  for (const name of names.filter((n) => !CODE(n, engine) && !n.endsWith('_bg.wasm.d.ts'))) cpSync(join(dist, name), join(licenses, name));
  cpSync(join(dist, 'provenance.json'), join(licenses, 'provenance.json'));
  console.log(`${engine}: ${provenance.tag} (${provenance.commit.slice(0, 7)}) synced`);
}
