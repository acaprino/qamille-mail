// Brings the toolkit (@acaprino/toolkit, the design system) into desktop/vendor/toolkit/, after checking every file
// of the package against its provenance.json. Output is generated and ignored by git.
// Qamille always takes the latest release of the toolkit repository's main branch, with no pin of its own:
//   the latest GitHub release of acaprino/toolkit, used only when its tag is on main (public, plain HTTPS),
//   cached in desktop/.cache/toolkit;
//   TOOLKIT_DIR, when set, takes its place: a checkout of the toolkit, where `npm run release -- --skip-test`
//   builds the package (to try changes that are not released yet);
//   without the network the copy already in vendor/toolkit stays.
// The official repository is the only source. Not part of predev/prebuild: nothing imports the toolkit yet.
import { execSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { isPlainFileName, latestMainRelease, untar } from './github-release.mjs';

export const TOOLKIT_REPO = 'acaprino/toolkit';
const TOOLKIT_PACKAGE = '@acaprino/toolkit';
// A peer pinned to one version (as opposed to a range such as "^18.0.0 || ^19.0.0" or "*").
const EXACT_VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
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
    else throw new Error(`toolkit: ${path} is not a plain file`);
  }
  return found;
}

// Checks the unpacked package in `dir` against its provenance.json: the same files, with the same SHA-256, no more
// and no fewer. Returns the provenance.
function verify(dir) {
  const file = join(dir, 'provenance.json');
  if (!existsSync(file)) throw new Error(`toolkit: ${dir} has no provenance.json`);
  const provenance = readJson(file);
  if (provenance.package !== TOOLKIT_PACKAGE) throw new Error(`toolkit: the package is ${JSON.stringify(provenance.package)}, not ${TOOLKIT_PACKAGE}`);
  const listed = provenance.files;
  if (!listed || typeof listed !== 'object' || typeof provenance.commit !== 'string') throw new Error('toolkit: provenance.json lacks files or commit');
  const present = new Set(filesOf(dir).filter((path) => path !== 'provenance.json'));
  const problems = [
    ...[...present].filter((path) => !Object.hasOwn(listed, path)).map((path) => `${path} is not in provenance.json`),
    ...Object.keys(listed).filter((path) => !present.has(path)).map((path) => `${path} is listed in provenance.json but missing`),
    ...[...present].filter((path) => Object.hasOwn(listed, path) && listed[path] !== sha256(join(dir, path))).map((path) => `${path} does not match provenance.json`),
  ];
  if (problems.length) throw new Error(`toolkit: the package does not match its provenance.json. Rebuild or download it again.\n  ${problems.join('\n  ')}`);
  return provenance;
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
    if (!EXACT_VERSION.test(spec)) log(`toolkit: peer ${name} ${spec} is a range, not enforced (Qamille has ${have[name]})`);
    else if (have[name] !== spec) mismatches.push(`peer ${name} is exactly ${spec} in the toolkit, but Qamille has ${have[name]}`);
  }
  if (mismatches.length) throw new Error(`toolkit: ${mismatches.join('; ')}. Align desktop/package.json with the toolkit, or the toolkit with Qamille.`);
}

// Replaces vendor/toolkit with the package in `dir`: copied to vendor/toolkit.part first, then renamed into place.
function replaceVendor(dir, desktop) {
  const vendor = join(desktop, 'vendor');
  const target = join(vendor, 'toolkit');
  const part = `${target}.part`;
  rmSync(part, { recursive: true, force: true });
  mkdirSync(vendor, { recursive: true });
  try {
    cpSync(dir, part, { recursive: true });
  } catch (error) {
    rmSync(part, { recursive: true, force: true });
    throw error;
  }
  rmSync(target, { recursive: true, force: true });
  renameSync(part, target);
}

// Builds the package in a checkout of the toolkit and unpacks it into `into`. The checkout's own release script writes
// release/: the .tgz and the manifest that gives its SHA-256.
function buildLocal(source, into, log) {
  if (!existsSync(join(source, 'package.json'))) throw new Error(`toolkit: TOOLKIT_DIR ${source} is not a checkout of the toolkit (no package.json)`);
  log(`toolkit: building the package in ${source} (npm run release -- --skip-test)`);
  execSync(`${NPM} run release -- --skip-test`, { cwd: source, stdio: 'inherit' });
  const releaseDir = join(source, 'release');
  const manifest = readJson(join(releaseDir, 'manifest.json'));
  const pkg = manifest.packages?.find((p) => p.name === TOOLKIT_PACKAGE);
  if (!pkg) throw new Error(`toolkit: ${join(releaseDir, 'manifest.json')} does not list ${TOOLKIT_PACKAGE}`);
  if (!isPlainFileName(pkg.file)) throw new Error(`toolkit: the manifest lists ${JSON.stringify(pkg.file)}, which is not a plain file name`);
  const file = join(releaseDir, pkg.file);
  if (sha256(file) !== pkg.sha256) throw new Error(`toolkit: ${pkg.file}: SHA-256 differs from the manifest`);
  rmSync(into, { recursive: true, force: true });
  untar(file, into);
}

// Verifies the package of `release` ({ tag, dir }: `dir` is the unpacked package, null when there is none), checks its
// peers and puts it in vendor/toolkit. Nothing is touched before the package and its peers are good.
function install(release, desktop, log) {
  const { dir, tag } = release ?? {};
  if (!dir) {
    if (existsSync(join(desktop, 'vendor', 'toolkit', 'provenance.json'))) {
      log("toolkit: no release of the toolkit's main reachable, keeping the existing copy");
      return;
    }
    throw new Error(`toolkit: no release of ${TOOLKIT_REPO}'s main reachable and no copy in vendor/toolkit. Check the network (the latest release of ${TOOLKIT_REPO}'s main), or set TOOLKIT_DIR to a checkout of the toolkit.`);
  }
  const provenance = verify(dir);
  checkPeers(dir, desktop, log);
  replaceVendor(dir, desktop);
  log(`toolkit: ${tag ?? `v${provenance.version}`} (${provenance.commit.slice(0, 7)}) synced`);
}

// desktop: Qamille's desktop folder. release: { tag, dir }, the latest release of the toolkit's main already
// downloaded, `dir` being the unpacked package (null when there is none). source: a checkout of the toolkit; it wins
// over `release`: its package is built there and takes the release's place.
export async function syncToolkit({ desktop, release = null, source = null, log = console.log }) {
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
// Resolves to { tag, dir } with dir the unpacked package, or null (no network, no release): syncToolkit then falls back
// to the copy it already has. `api` is where GitHub's API is; only the tests replace it.
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
  return found && { tag: found.tag, dir: join(found.dir, 'package') };
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
