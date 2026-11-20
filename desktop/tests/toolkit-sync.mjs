// scripts/sync-toolkit.mjs: bringing the toolkit into vendor/toolkit/. The package is a fake one built here, like the
// ones a toolkit release holds: its files, a package.json and the provenance.json that lists every other file with its
// SHA-256. A release arrives unpacked ({ tag, dir }, what latestMainRelease finds); TOOLKIT_DIR is a fake checkout
// whose `npm run release` leaves a real .tgz (built with the system tar) and its manifest in release/.
// The scratch folders are under desktop/.cache (ignored by git), not in the system's temp folder.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { latestToolkitRelease, syncToolkit } from '../scripts/sync-toolkit.mjs';

const TAR = process.platform === 'win32' ? join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
const cache = join(import.meta.dirname, '..', '.cache');
mkdirSync(cache, { recursive: true });
const scratch = mkdtempSync(join(cache, 'test-toolkit-sync-'));
test.after(() => rmSync(scratch, { recursive: true, force: true }));
let counter = 0;
const fresh = (name) => join(scratch, `${name}-${++counter}`);

const sha256 = (data) => createHash('sha256').update(data).digest('hex');
const COMMIT = '0123456789abcdef0123456789abcdef01234567';
const write = (file, text) => {
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, text);
};

// A toolkit package, unpacked: <dir>/package.json, the files and provenance.json. The folder is called "package", like
// the top folder of the .tgz. `files` adds to (or, with a different text, replaces) the default ones.
function toolkitPackage({ peerDependencies, version = '1.2.3', commit = COMMIT, files = {}, name = '@acaprino/toolkit' } = {}) {
  const dir = join(fresh('pkg'), 'package');
  const all = {
    'package.json': `${JSON.stringify({ name, version, peerDependencies }, null, 2)}\n`,
    'dist/index.js': 'export const toolkit = 1;\n',
    'dist/index.d.ts': 'export declare const toolkit: 1;\n',
    'src/styles/theme.css': ':root { color-scheme: light dark; }\n',
    LICENSE: 'MIT\n',
    ...files,
  };
  for (const [path, text] of Object.entries(all)) write(join(dir, path), text);
  const provenance = { package: name, version, commit, files: Object.fromEntries(Object.keys(all).sort().map((path) => [path, sha256(all[path])])) };
  write(join(dir, 'provenance.json'), `${JSON.stringify(provenance, null, 2)}\n`);
  return dir;
}

// The desktop folder of Qamille: its package.json, and optionally a copy of the toolkit synced earlier.
function desktopOf({ dependencies = { react: '19.3.1' }, devDependencies = {}, old = false } = {}) {
  const desktop = fresh('desktop');
  write(join(desktop, 'package.json'), `${JSON.stringify({ name: 'qamille-desktop', dependencies, devDependencies }, null, 2)}\n`);
  if (old) {
    write(join(desktop, 'vendor', 'toolkit', 'old.txt'), 'old\n');
    write(join(desktop, 'vendor', 'toolkit', 'provenance.json'), '{ "version": "0.9.0" }\n');
  }
  return desktop;
}

// Every file under a folder, with its content: to compare a folder with another, or with itself before and after.
function tree(dir, prefix = '') {
  const found = {};
  for (const entry of readdirSync(join(dir, prefix), { withFileTypes: true })) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) Object.assign(found, tree(dir, path));
    else found[path] = readFileSync(join(dir, path), 'utf8');
  }
  return found;
}

const vendorOf = (desktop) => join(desktop, 'vendor', 'toolkit');
const lines = () => { const list = []; list.log = (line) => list.push(line); return list; };
const releaseOf = (dir, tag = 'v1.2.3') => ({ tag, dir });
const synced = (tag = 'v1.2.3') => `toolkit: ${tag} (${COMMIT.slice(0, 7)}) synced`;

// The old copy is untouched and no staging folder is left.
const assertKept = (desktop, before) => {
  assert.deepEqual(tree(vendorOf(desktop)), before);
  assert.equal(existsSync(`${vendorOf(desktop)}.part`), false);
};

test('a verified package lands in vendor/toolkit', async () => {
  const dir = toolkitPackage();
  const desktop = desktopOf({ old: true });
  const log = lines();
  await syncToolkit({ desktop, release: releaseOf(dir), log: log.log });
  // The whole package, provenance.json included, and nothing of the earlier copy.
  assert.deepEqual(tree(vendorOf(desktop)), tree(dir));
  assert.ok('provenance.json' in tree(vendorOf(desktop)));
  assert.equal(existsSync(join(vendorOf(desktop), 'old.txt')), false);
  assert.equal(existsSync(`${vendorOf(desktop)}.part`), false);
  assert.deepEqual([...log], [synced()]);
});

test('the first sync creates vendor/toolkit', async () => {
  const dir = toolkitPackage();
  const desktop = desktopOf();
  assert.equal(existsSync(join(desktop, 'vendor')), false);
  await syncToolkit({ desktop, release: releaseOf(dir), log: () => {} });
  assert.deepEqual(tree(vendorOf(desktop)), tree(dir));
});

test('a file that differs from provenance.json is refused and the old copy stays', async () => {
  const dir = toolkitPackage();
  write(join(dir, 'dist', 'index.js'), 'export const toolkit = "tampered";\n');
  const desktop = desktopOf({ old: true });
  const before = tree(vendorOf(desktop));
  const log = lines();
  await assert.rejects(syncToolkit({ desktop, release: releaseOf(dir), log: log.log }), /dist\/index\.js does not match provenance\.json/);
  assertKept(desktop, before);
  assert.deepEqual([...log], []);
});

test('a file that provenance.json does not list is refused and the old copy stays', async () => {
  const dir = toolkitPackage();
  write(join(dir, 'dist', 'extra.js'), 'console.log("extra");\n');
  const desktop = desktopOf({ old: true });
  const before = tree(vendorOf(desktop));
  await assert.rejects(syncToolkit({ desktop, release: releaseOf(dir), log: () => {} }), /dist\/extra\.js is not in provenance\.json/);
  assertKept(desktop, before);
});

test('a file that provenance.json lists but the package lacks is refused and the old copy stays', async () => {
  const dir = toolkitPackage();
  rmSync(join(dir, 'src', 'styles', 'theme.css'));
  const desktop = desktopOf({ old: true });
  const before = tree(vendorOf(desktop));
  await assert.rejects(syncToolkit({ desktop, release: releaseOf(dir), log: () => {} }), /src\/styles\/theme\.css is listed in provenance\.json but missing/);
  assertKept(desktop, before);
});

test('a package without provenance.json, or that is not the toolkit, is refused', async () => {
  const bare = toolkitPackage();
  rmSync(join(bare, 'provenance.json'));
  const other = toolkitPackage({ name: 'left-pad' });
  const desktop = desktopOf({ old: true });
  const before = tree(vendorOf(desktop));
  await assert.rejects(syncToolkit({ desktop, release: releaseOf(bare), log: () => {} }), /no provenance\.json/);
  await assert.rejects(syncToolkit({ desktop, release: releaseOf(other), log: () => {} }), /"left-pad", not @acaprino\/toolkit/);
  assertKept(desktop, before);
});

test('an exact peer that differs from Qamille stops the sync', async () => {
  const dir = toolkitPackage({ peerDependencies: { react: '19.0.0' } });
  const desktop = desktopOf({ dependencies: { react: '19.3.1' }, old: true });
  const before = tree(vendorOf(desktop));
  const log = lines();
  await assert.rejects(syncToolkit({ desktop, release: releaseOf(dir), log: log.log }), (error) => {
    assert.match(error.message, /react/);
    assert.match(error.message, /19\.0\.0/);
    assert.match(error.message, /19\.3\.1/);
    return true;
  });
  assertKept(desktop, before);
  assert.deepEqual([...log], []);
});

test('an exact peer that equals Qamille passes, whether it is a dependency or a devDependency', async () => {
  const dir = toolkitPackage({ peerDependencies: { react: '19.3.1', 'react-dom': '19.3.1' } });
  const desktop = desktopOf({ dependencies: { react: '19.3.1' }, devDependencies: { 'react-dom': '19.3.1' } });
  const log = lines();
  await syncToolkit({ desktop, release: releaseOf(dir), log: log.log });
  assert.deepEqual([...log], [synced()], 'an enforced peer that holds needs no line');
  assert.deepEqual(tree(vendorOf(desktop)), tree(dir));
});

test('a range peer is reported, not enforced', async () => {
  const dir = toolkitPackage({ peerDependencies: { react: '^18.0.0 || ^19.0.0', 'react-dom': '^18.0.0', 'solid-js': '^1.0.0' } });
  // react-dom is far from the range and still passes; solid-js is not Qamille's at all, so it is not mentioned.
  const desktop = desktopOf({ dependencies: { react: '19.3.1' }, devDependencies: { 'react-dom': '17.0.2' } });
  const log = lines();
  await syncToolkit({ desktop, release: releaseOf(dir), log: log.log });
  assert.deepEqual([...log], [
    'toolkit: peer react ^18.0.0 || ^19.0.0 is a range, not enforced (Qamille has 19.3.1)',
    'toolkit: peer react-dom ^18.0.0 is a range, not enforced (Qamille has 17.0.2)',
    synced(),
  ]);
  assert.deepEqual(tree(vendorOf(desktop)), tree(dir));
});

test('no release and no copy fails with a useful message', async () => {
  const desktop = desktopOf();
  const log = lines();
  await assert.rejects(syncToolkit({ desktop, release: null, log: log.log }), (error) => {
    assert.match(error.message, /network/);
    assert.match(error.message, /TOOLKIT_DIR/);
    assert.match(error.message, /acaprino\/toolkit/);
    return true;
  });
  assert.equal(existsSync(vendorOf(desktop)), false);
});

test('no release but a copy keeps the copy', async () => {
  const desktop = desktopOf({ old: true });
  const before = tree(vendorOf(desktop));
  const log = lines();
  await syncToolkit({ desktop, release: null, log: log.log });
  assert.deepEqual([...log], ["toolkit: no release of the toolkit's main reachable, keeping the existing copy"]);
  assertKept(desktop, before);
});

// The package folder as a .tgz with one top folder, "package", written to `out` (the system tar, as in a release).
function packTgz(packageDir, out) {
  mkdirSync(dirname(out), { recursive: true });
  execFileSync(TAR, ['-czf', out, '-C', dirname(packageDir), 'package'], { stdio: 'ignore' });
  return out;
}

// TOOLKIT_DIR: a fake checkout. Its `npm run release` copies a prebuilt release/ (the .tgz and its manifest) into
// place and records the arguments it got, as the toolkit's scripts/release.mjs would leave release/.
function fakeCheckout({ manifestSha = null, packageDir = toolkitPackage() } = {}) {
  const checkout = fresh('checkout');
  const tgz = packTgz(packageDir, join(checkout, 'prebuilt', 'acaprino-toolkit-1.2.3.tgz'));
  const manifest = { version: '1.2.3', packages: [{ name: '@acaprino/toolkit', file: 'acaprino-toolkit-1.2.3.tgz', sha256: manifestSha ?? sha256(readFileSync(tgz)) }] };
  write(join(checkout, 'prebuilt', 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  write(join(checkout, 'package.json'), `${JSON.stringify({ name: 'fake-toolkit', scripts: { release: 'node make-release.mjs' } })}\n`);
  write(join(checkout, 'make-release.mjs'), [
    "import { cpSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';",
    "rmSync('release', { recursive: true, force: true });",
    "mkdirSync('release');",
    "cpSync('prebuilt', 'release', { recursive: true });",
    "writeFileSync('release-args.json', JSON.stringify(process.argv.slice(2)));",
    '',
  ].join('\n'));
  return { checkout, packageDir };
}

test('TOOLKIT_DIR builds the package in the checkout and takes it from release/', async () => {
  const { checkout, packageDir } = fakeCheckout();
  const desktop = desktopOf({ old: true });
  const log = lines();
  await syncToolkit({ desktop, source: checkout, log: log.log });
  assert.deepEqual(JSON.parse(readFileSync(join(checkout, 'release-args.json'), 'utf8')), ['--skip-test']);
  assert.deepEqual(tree(vendorOf(desktop)), tree(packageDir));
  assert.equal(log.at(-1), synced());
  assert.equal(existsSync(`${vendorOf(desktop)}.part`), false);
  assert.equal(existsSync(join(desktop, '.cache', 'toolkit-local')), false, 'the staging folder is removed');
});

test('TOOLKIT_DIR wins over a release that was found', async () => {
  const { checkout, packageDir } = fakeCheckout();
  const found = toolkitPackage({ version: '9.9.9' });
  const desktop = desktopOf();
  await syncToolkit({ desktop, release: releaseOf(found, 'v9.9.9'), source: checkout, log: () => {} });
  assert.deepEqual(tree(vendorOf(desktop)), tree(packageDir));
});

test('TOOLKIT_DIR: a package that differs from its manifest is refused and the old copy stays', async () => {
  const { checkout } = fakeCheckout({ manifestSha: '0'.repeat(64) });
  const desktop = desktopOf({ old: true });
  const before = tree(vendorOf(desktop));
  await assert.rejects(syncToolkit({ desktop, source: checkout, log: () => {} }), /SHA-256 differs from the manifest/);
  assertKept(desktop, before);
});

test('TOOLKIT_DIR: a package that differs from its provenance.json is refused and the old copy stays', async () => {
  const packageDir = toolkitPackage();
  write(join(packageDir, 'dist', 'index.js'), 'export const toolkit = "tampered";\n');
  const { checkout } = fakeCheckout({ packageDir });
  const desktop = desktopOf({ old: true });
  const before = tree(vendorOf(desktop));
  await assert.rejects(syncToolkit({ desktop, source: checkout, log: () => {} }), /dist\/index\.js does not match provenance\.json/);
  assertKept(desktop, before);
  assert.equal(existsSync(join(desktop, '.cache', 'toolkit-local')), false, 'the staging folder is removed, refused or not');
});

test('TOOLKIT_DIR that is not a checkout of the toolkit is refused', async () => {
  const desktop = desktopOf({ old: true });
  const before = tree(vendorOf(desktop));
  await assert.rejects(syncToolkit({ desktop, source: fresh('nothing-here'), log: () => {} }), /no package\.json/);
  assertKept(desktop, before);
});

// The release path end to end, with a local server standing in for GitHub: releases/latest, compare and the assets.
// The manifest lists another package first, which is not served: only the toolkit's own is fetched.
test('the latest release of the toolkit is downloaded, verified and synced', async () => {
  const packageDir = toolkitPackage({ peerDependencies: { react: '^19.0.0' } });
  const tgz = readFileSync(packTgz(packageDir, join(fresh('release'), 'acaprino-toolkit-1.2.3.tgz')));
  const manifest = { version: '1.2.3', packages: [
    { name: '@acaprino/other', file: 'acaprino-other-1.2.3.tgz', sha256: '0'.repeat(64) },
    { name: '@acaprino/toolkit', file: 'acaprino-toolkit-1.2.3.tgz', sha256: sha256(tgz) },
  ] };
  const repo = '/repos/acaprino/toolkit';
  const requests = [];
  let api = '';
  const server = createServer((req, res) => {
    requests.push(req.url);
    const send = (body, type = 'application/json') => { res.writeHead(200, { 'Content-Type': type }); res.end(body); };
    if (req.url === `${repo}/releases/latest`) {
      send(JSON.stringify({ tag_name: 'v1.2.3', assets: [
        { name: 'manifest.json', url: `${api}/asset/manifest` },
        { name: 'acaprino-toolkit-1.2.3.tgz', url: `${api}/asset/package` },
      ] }));
    } else if (req.url === `${repo}/compare/main...v1.2.3`) send(JSON.stringify({ status: 'behind' }));
    else if (req.url === '/asset/manifest') send(JSON.stringify(manifest));
    else if (req.url === '/asset/package') send(tgz, 'application/octet-stream');
    else { res.writeHead(404); res.end('not found'); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  api = `http://127.0.0.1:${server.address().port}`;
  try {
    const cacheDir = fresh('cache');
    const desktop = desktopOf({ dependencies: { react: '19.3.1' }, old: true });
    const log = lines();
    const release = await latestToolkitRelease(cacheDir, log.log, api);
    assert.deepEqual(release, { tag: 'v1.2.3', dir: join(cacheDir, 'v1.2.3', 'package') });
    assert.deepEqual(requests.filter((url) => url.startsWith('/asset/')), ['/asset/manifest', '/asset/package']);
    await syncToolkit({ desktop, release, log: log.log });
    assert.deepEqual(tree(vendorOf(desktop)), tree(packageDir));
    assert.deepEqual([...log], [
      'toolkit: v1.2.3 downloaded from acaprino/toolkit',
      'toolkit: peer react ^19.0.0 is a range, not enforced (Qamille has 19.3.1)',
      synced(),
    ]);
    // The cache holds the release for the next run, with its marker beside the package.
    assert.deepEqual(readdirSync(join(cacheDir, 'v1.2.3')).sort(), ['.complete', 'package']);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

test('a release whose manifest lacks the toolkit is not used', async () => {
  const server = createServer((req, res) => {
    const send = (body) => { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(body); };
    if (req.url.endsWith('/releases/latest')) send(JSON.stringify({ tag_name: 'v1.2.3', assets: [{ name: 'manifest.json', url: `${api}/asset/manifest` }] }));
    else if (req.url.includes('/compare/')) send(JSON.stringify({ status: 'identical' }));
    else if (req.url === '/asset/manifest') send(JSON.stringify({ version: '1.2.3', packages: [{ name: '@acaprino/other', file: 'o.tgz', sha256: '0'.repeat(64) }] }));
    else { res.writeHead(404); res.end('not found'); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const api = `http://127.0.0.1:${server.address().port}`;
  try {
    const log = lines();
    assert.equal(await latestToolkitRelease(fresh('cache'), log.log, api), null);
    assert.deepEqual([...log], ['toolkit: v1.2.3 could not be downloaded (@acaprino/toolkit is not in the manifest of v1.2.3)']);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});
