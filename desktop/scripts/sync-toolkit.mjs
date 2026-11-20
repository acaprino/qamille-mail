// Brings the toolkit (@acaprino/toolkit, the design system) into desktop/vendor/toolkit/, after checking every file
// of the package against its provenance.json. Output is generated and ignored by git.
// Qamille always takes the latest release of the toolkit repository's main branch, with no pin of its own:
//   the latest GitHub release of acaprino/toolkit, used only when its tag is on main (public, plain HTTPS),
//   cached in desktop/.cache/toolkit (a release is refused unless its tag is v<version of its provenance.json>, and a
//   cached release that fails its checks is deleted from the cache, so the next run downloads it again);
//   TOOLKIT_DIR, when set, takes its place: a checkout of the toolkit, where `npm run release -- --skip-test`
//   builds the package (to try changes that are not released yet);
//   without the network the copy already in vendor/toolkit stays, if it still matches its own provenance.json.
// The copy is replaced without ever being lost: the new one is written to vendor/toolkit.part, the old one is renamed
// to vendor/toolkit.old, the new one is renamed into place, and the old one is deleted last. If a sync stops half-way,
// the next one puts vendor/toolkit.old back.
// The official repository is the only source. Not part of predev/prebuild: nothing imports the toolkit yet.
import { execSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { isPlainFileName, latestMainRelease, untar } from './github-release.mjs';

export const TOOLKIT_REPO = 'acaprino/toolkit';
const TOOLKIT_PACKAGE = '@acaprino/toolkit';
// The toolkit tags its releases <prefix><version> (tagPrefix in its .github/release.json): v1.2.3.
const TOOLKIT_TAG_PREFIX = 'v';
// A peer pinned to one version (as opposed to a range such as "^18.0.0 || ^19.0.0" or "*"): "1.2.3", and also
// "=1.2.3" and "v1.2.3", which npm reads the same way (`bare` takes the prefix off).
const EXACT_VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const bare = (spec) => String(spec).trim().replace(/^=?\s*v?/, '');
// On Windows npm is npm.cmd, which Node runs only through a shell; the command line is fixed words.
const NPM = process.platform === 'win32' ? 'npm.cmd' : 'npm';

const sha256 = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
const readJson = (file) => JSON.parse(readFileSync(file, 'utf8'));

// Every file under `dir` as a forward-slash path relative to it. The package comes off the network, so anything
// that is not a plain file or folder (a link) is refused.
function filesOf(dir, prefix = '') {
  const found = [];
  for (const entry of readdirSync(join(dir, prefix), { withFileTypes: true })) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) found.push(...filesOf(dir, path));
    else if (entry.isFile()) found.push(path);
    else throw new Error(`${path} is not a plain file`);
  }
  return found;
}

// Checks the unpacked package in `dir` against its provenance.json: the same files, with the same SHA-256, no more
// and no fewer. Returns { provenance, problems }: what is wrong with the package, an empty list when it is good.
// It serves a new package and the copy already in vendor/toolkit alike.
function inspect(dir) {
  const file = join(dir, 'provenance.json');
  if (!existsSync(file)) return { provenance: null, problems: ['no provenance.json'] };
  let provenance;
  try {
    provenance = readJson(file);
  } catch (error) {
    return { provenance: null, problems: [`provenance.json cannot be read (${error.message})`] };
  }
  if (provenance?.package !== TOOLKIT_PACKAGE) return { provenance, problems: [`the package is ${JSON.stringify(provenance?.package)}, not ${TOOLKIT_PACKAGE}`] };
  const listed = provenance.files;
  if (!listed || typeof listed !== 'object' || typeof provenance.commit !== 'string') return { provenance, problems: ['provenance.json lacks files or commit'] };
  let present;
  try {
    present = new Set(filesOf(dir).filter((path) => path !== 'provenance.json'));
  } catch (error) {
    return { provenance, problems: [error.message] };
  }
  const problems = [
    ...[...present].filter((path) => !Object.hasOwn(listed, path)).map((path) => `${path} is not in provenance.json`),
    ...Object.keys(listed).filter((path) => !present.has(path)).map((path) => `${path} is listed in provenance.json but missing`),
    ...[...present].filter((path) => Object.hasOwn(listed, path) && listed[path] !== sha256(join(dir, path))).map((path) => `${path} does not match provenance.json`),
  ];
  return { provenance, problems };
}

// A new package that fails the check is an error. Returns its provenance. `tag` is the tag of the release the package
// came from (none for a package built in TOOLKIT_DIR): the release of tag v1.2.3 must hold version 1.2.3, or a package
// uploaded to the wrong release would be synced under that tag.
function verify(dir, tag) {
  const { provenance, problems } = inspect(dir);
  if (problems.length) throw new Error(`toolkit: the package fails its checks against provenance.json. Rebuild or download it again.\n  ${problems.join('\n  ')}`);
  const version = typeof provenance.version === 'string' ? provenance.version : null;
  if (tag !== undefined && (version === null || tag !== `${TOOLKIT_TAG_PREFIX}${version}`)) {
    const belongs = version === null ? '' : `, which is the tag ${TOOLKIT_TAG_PREFIX}${version}`;
    throw new Error(`toolkit: the release ${tag} holds a package whose provenance.json says version ${JSON.stringify(version)}${belongs}. A package was uploaded to the wrong release.`);
  }
  return provenance;
}

// A release from the cache that fails its checks is taken out of the cache: its .complete marker was written before the
// checks, so without this the next run would trust it and never download it again. Returns the sentence that says so
// (nothing when the package did not come from the cache).
function dropCached(cached, tag) {
  if (!cached) return '';
  try {
    rmSync(cached, { recursive: true, force: true });
  } catch (error) {
    return `\n  The cached copy of ${tag} could not be removed (${error.message}); delete ${cached} by hand.`;
  }
  return `\n  The cached copy of ${tag} was removed, so the next run downloads it again.`;
}

// The toolkit's peerDependencies that Qamille also declares. A peer pinned to an exact version must be the version
// Qamille has, or the toolkit would run on another copy of it than it was built for; a range is only reported.
function checkPeers(dir, desktop, log) {
  const peers = readJson(join(dir, 'package.json')).peerDependencies ?? {};
  const own = readJson(join(desktop, 'package.json'));
  const have = { ...own.devDependencies, ...own.dependencies };
  const mismatches = [];
  for (const [name, spec] of Object.entries(peers)) {
    if (!Object.hasOwn(have, name)) continue;
    if (!EXACT_VERSION.test(bare(spec))) log(`toolkit: peer ${name} ${spec} is a range, not enforced (Qamille has ${have[name]})`);
    else if (bare(have[name]) !== bare(spec)) mismatches.push(`peer ${name} is exactly ${spec} in the toolkit, but Qamille has ${have[name]}`);
  }
  if (mismatches.length) throw new Error(`toolkit: ${mismatches.join('; ')}. Align desktop/package.json with the toolkit, or the toolkit with Qamille.`);
}

// Windows answers EPERM, EACCES or EBUSY to a rename while something (the antivirus, the indexer) still holds a file
// of a folder that was just written: the rename is tried again for a moment (1.5 s in all) before it is given up.
const TRANSIENT = new Set(['EPERM', 'EACCES', 'EBUSY']);
const pause = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
export function renameRetrying(from, to, { rename = renameSync, wait = pause, attempts = 6 } = {}) {
  for (let attempt = 1; ; attempt++) {
    try {
      return rename(from, to);
    } catch (error) {
      if (attempt >= attempts || !TRANSIENT.has(error.code)) throw error;
      wait(100 * attempt);
    }
  }
}

// Replaces vendor/toolkit with the package in `dir`, with no moment in which the copy exists nowhere: the package is
// copied to vendor/toolkit.part, the old copy is renamed to vendor/toolkit.old, the .part is renamed into place, and
// the old copy is deleted last (that deletion cannot fail the sync: the next one removes what is left). If the new
// copy cannot be put in place the old one is renamed back; if even that fails it stays in toolkit.old, and
// restoreInterrupted brings it back at the next sync. `rename` is where the tests make the renames fail.
export function replaceVendor(dir, desktop, rename = renameRetrying) {
  const vendor = join(desktop, 'vendor');
  const target = join(vendor, 'toolkit');
  const part = `${target}.part`;
  const old = `${target}.old`;
  rmSync(part, { recursive: true, force: true });
  mkdirSync(vendor, { recursive: true });
  try {
    cpSync(dir, part, { recursive: true });
  } catch (error) {
    rmSync(part, { recursive: true, force: true });
    throw error;
  }
  let aside = false;
  try {
    if (existsSync(target)) {
      rmSync(old, { recursive: true, force: true });
      rename(target, old);
      aside = true;
    }
    rename(part, target);
  } catch (error) {
    rmSync(part, { recursive: true, force: true });
    if (aside) {
      try {
        rename(old, target);
      } catch (restoreError) {
        throw new Error(`${error.message}. The previous copy is in ${old} and the next sync puts it back (${restoreError.message})`);
      }
    }
    throw error;
  }
  try {
    rmSync(old, { recursive: true, force: true });
  } catch {
    // Left over, harmless: restoreInterrupted removes it next time.
  }
}

// A sync that stopped after putting the old copy aside and before the new one was in place leaves vendor/toolkit.old
// and no vendor/toolkit: that is the last good copy, and it goes back. A toolkit.old beside a vendor/toolkit is what
// a sync left when it stopped before deleting it, and is removed.
function restoreInterrupted(desktop, log) {
  const target = join(desktop, 'vendor', 'toolkit');
  const old = `${target}.old`;
  if (!existsSync(old)) return;
  if (existsSync(target)) {
    try {
      rmSync(old, { recursive: true, force: true });
    } catch {
      // A locked file: the next replacement tries again.
    }
    return;
  }
  renameRetrying(old, target);
  log('toolkit: vendor/toolkit was missing, the previous copy of an interrupted sync is back');
}

// Builds the package in a checkout of the toolkit and unpacks it into `into`. The checkout's own release script writes
// release/: the .tgz and the manifest that gives its SHA-256.
function buildLocal(source, into, log) {
  if (!existsSync(join(source, 'package.json'))) throw new Error(`toolkit: TOOLKIT_DIR ${source} is not a checkout of the toolkit (no package.json)`);
  log(`toolkit: building the package in ${source} (npm run release -- --skip-test)`);
  try {
    execSync(`${NPM} run release -- --skip-test`, { cwd: source, stdio: 'inherit' });
  } catch (error) {
    const how = typeof error.status === 'number' ? `exit code ${error.status}` : error.message;
    throw new Error(`toolkit: npm run release -- --skip-test failed in TOOLKIT_DIR ${source} (${how})`);
  }
  const releaseDir = join(source, 'release');
  if (!existsSync(join(releaseDir, 'manifest.json'))) throw new Error(`toolkit: npm run release left no release/manifest.json in TOOLKIT_DIR ${source}`);
  const manifest = readJson(join(releaseDir, 'manifest.json'));
  const pkg = manifest.packages?.find((p) => p.name === TOOLKIT_PACKAGE);
  if (!pkg) throw new Error(`toolkit: ${join(releaseDir, 'manifest.json')} does not list ${TOOLKIT_PACKAGE}`);
  if (!isPlainFileName(pkg.file)) throw new Error(`toolkit: the manifest lists ${JSON.stringify(pkg.file)}, which is not a plain file name`);
  const file = join(releaseDir, pkg.file);
  if (sha256(file) !== pkg.sha256) throw new Error(`toolkit: ${pkg.file}: SHA-256 differs from the manifest`);
  rmSync(into, { recursive: true, force: true });
  untar(file, into);
}

// Verifies the package of `release` ({ tag, dir, cached }: `dir` is the unpacked package, null when there is none; `cached`
// is the folder of that release in the cache, when it came from there), checks its peers and puts it in vendor/toolkit.
// Nothing is touched before the package and its peers are good. A cached release that fails its checks is removed from
// the cache. Without a package the copy in vendor/toolkit stays, but only if it is whole: it is checked against its own
// provenance.json, like a new one.
function install(release, desktop, log) {
  const { dir, tag, cached } = release ?? {};
  if (!dir) {
    const advice = `Check the network (the latest release of ${TOOLKIT_REPO}'s main), or set TOOLKIT_DIR to a checkout of the toolkit.`;
    const kept = join(desktop, 'vendor', 'toolkit');
    if (!existsSync(kept)) throw new Error(`toolkit: no release of ${TOOLKIT_REPO}'s main reachable and no copy in vendor/toolkit. ${advice}`);
    const { problems } = inspect(kept);
    if (problems.length) throw new Error(`toolkit: no release of ${TOOLKIT_REPO}'s main reachable, and the copy in vendor/toolkit fails its checks against provenance.json. ${advice}\n  ${problems.join('\n  ')}`);
    log("toolkit: no release of the toolkit's main reachable, keeping the existing copy");
    return;
  }
  let provenance;
  try {
    provenance = verify(dir, tag);
  } catch (error) {
    error.message += dropCached(cached, tag);
    throw error;
  }
  checkPeers(dir, desktop, log);
  replaceVendor(dir, desktop);
  log(`toolkit: ${tag ?? `v${provenance.version}`} (${provenance.commit.slice(0, 7)}) synced`);
}

// desktop: Qamille's desktop folder. release: { tag, dir, cached }, the latest release of the toolkit's main already
// downloaded, `dir` being the unpacked package (null when there is none) and `cached` the cache folder of the release
// (removed when the package fails its checks; optional). source: a checkout of the toolkit; it wins
// over `release`: its package is built there and takes the release's place.
export async function syncToolkit({ desktop, release = null, source = null, log = console.log }) {
  restoreInterrupted(desktop, log);
  if (!source) return install(release, desktop, log);
  // The checkout's package is unpacked into a staging folder of its own, removed at the end.
  const staging = join(desktop, '.cache', 'toolkit-local');
  try {
    buildLocal(source, staging, log);
    install({ dir: staging }, desktop, log);
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}

// The latest release of the toolkit's main (see github-release.mjs), its one package unpacked into <cache>/<tag>/package/.
// Resolves to { tag, dir, cached } with dir the unpacked package and cached the release's folder in the cache, or null
// (no network, no release): syncToolkit then falls back to the copy it already has. `api` is where GitHub's API is; only
// the tests replace it.
export async function latestToolkitRelease(cache, log = console.log, api) {
  const found = await latestMainRelease({
    repo: TOOLKIT_REPO,
    cache,
    label: 'toolkit',
    log,
    api,
    select: (manifest, tag) => [manifest.packages?.find((p) => p.name === TOOLKIT_PACKAGE) ?? (() => { throw new Error(`${TOOLKIT_PACKAGE} is not in the manifest of ${tag}`); })()],
    unpack: (file, part) => untar(file, join(part, 'package')),
  });
  return found && { tag: found.tag, dir: join(found.dir, 'package'), cached: found.dir };
}

// Command line: `npm run toolkit:sync`. A failure is reported and sets the exit code; the process is left to end by
// itself, because on Windows an uncaught error right after a request leaves Node aborting on exit (0xC0000409).
async function main() {
  const desktop = resolve(import.meta.dirname, '..');
  const explicit = process.env.TOOLKIT_DIR;
  const release = explicit ? null : await latestToolkitRelease(join(desktop, '.cache', 'toolkit'));
  await syncToolkit({ desktop, release, source: explicit ? resolve(explicit) : null });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
