// The shared GitHub release download (scripts/github-release.mjs) and ArtCraft's use of it. A local HTTP server
// stands in for GitHub: releases/latest, compare and the asset URLs that releases/latest returns. The packages are
// real .tgz files, built with the system tar.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import test from 'node:test';
import { ARTCRAFT_ENGINES, ARTCRAFT_REPO, latestArtcraftRelease } from '../scripts/artcraft-github.mjs';
import { latestMainRelease, untar } from '../scripts/github-release.mjs';

const TAR = process.platform === 'win32' ? join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
const scratch = mkdtempSync(join(tmpdir(), 'qamille-github-release-'));
test.after(() => rmSync(scratch, { recursive: true, force: true }));
let counter = 0;
const fresh = (name) => join(scratch, `${name}-${++counter}`);

// A package like the ones in a release: one top folder (package/) holding the files.
function tgz(files) {
  const root = fresh('pkg');
  for (const [path, text] of Object.entries(files)) {
    const file = join(root, 'src', 'package', path);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, text);
  }
  const out = join(root, 'package.tgz');
  execFileSync(TAR, ['-czf', out, '-C', join(root, 'src'), 'package'], { stdio: 'ignore' });
  return readFileSync(out);
}

// A package of the release: its file name, its content and extra manifest fields. `sha256` overrides the digest.
const pack = (file, files, extra = {}) => ({ file, data: tgz(files), extra });
const sha256 = (data) => createHash('sha256').update(data).digest('hex');

// A GitHub that serves one repository: releases/latest names `tag`, compare answers `status`, and the assets are the
// manifest.json plus the packages, at API-style asset URLs. `requests` lists what was asked, in order.
async function fakeGitHub({ repo, tag, status = 'behind', packages, manifestPackages }) {
  const requests = [];
  const assets = new Map();
  const names = [];
  let api = '';
  const add = (name, data) => { names.push(name); assets.set(String(assets.size + 1), { name, data }); };
  const entries = manifestPackages ?? packages.map(({ file, data, extra }) => ({ ...extra, file, sha256: extra.sha256 ?? sha256(data) }));
  add('manifest.json', Buffer.from(JSON.stringify({ version: tag.slice(1), packages: entries })));
  for (const { file, data } of packages) add(file, data);
  const server = createServer((req, res) => {
    const send = (code, body, type) => { res.writeHead(code, { 'Content-Type': type }); res.end(body); };
    const asset = /^\/repos\/[^/]+\/[^/]+\/releases\/assets\/(\d+)$/.exec(req.url)?.[1];
    if (req.url === `/repos/${repo}/releases/latest`) {
      requests.push({ url: req.url, authorization: req.headers.authorization });
      const list = [...assets].map(([id, { name }]) => ({ name, url: `${api}/repos/${repo}/releases/assets/${id}` }));
      send(200, JSON.stringify({ tag_name: tag, assets: list }), 'application/json');
    } else if (req.url === `/repos/${repo}/compare/main...${tag}`) {
      requests.push({ url: req.url, authorization: req.headers.authorization });
      send(200, JSON.stringify({ status }), 'application/json');
    } else if (asset && assets.has(asset)) {
      const { name, data } = assets.get(asset);
      requests.push({ url: `asset:${name}`, authorization: req.headers.authorization });
      send(200, data, 'application/octet-stream');
    } else {
      send(404, 'not found', 'text/plain');
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  api = `http://127.0.0.1:${server.address().port}`;
  return {
    api,
    requests: () => requests.map((r) => r.url),
    authorizations: () => requests.map((r) => r.authorization),
    names,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

// An address nobody listens on: the "no network" of the tests.
async function closedApi() {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return `http://127.0.0.1:${port}`;
}

const REPO = 'acme/thing';
const thingPackage = () => pack('thing-1.2.3.tgz', { 'hello.txt': 'hello\n', 'sub/inner.txt': 'inner\n' });
const lines = () => { const list = []; list.log = (line) => list.push(line); return list; };

// Unpacks the package under <part>/out and records the calls.
const unpacker = () => {
  const calls = [];
  return { calls, unpack: (file, part, pkg) => { calls.push({ name: basename(file), dir: dirname(file), part, pkg }); untar(file, join(part, 'out')); } };
};

test('a release on main is downloaded, verified and unpacked', async () => {
  const github = await fakeGitHub({ repo: REPO, tag: 'v1.2.3', packages: [thingPackage()] });
  const cache = fresh('cache');
  mkdirSync(join(cache, 'v1.0.0'), { recursive: true });
  const log = lines();
  const { calls, unpack } = unpacker();
  try {
    const found = await latestMainRelease({ repo: REPO, cache, label: 'thing', unpack, log: log.log, api: github.api });
    const dir = join(cache, 'v1.2.3');
    assert.deepEqual(found, { tag: 'v1.2.3', dir });
    assert.equal(readFileSync(join(dir, 'out', 'hello.txt'), 'utf8'), 'hello\n');
    assert.equal(readFileSync(join(dir, 'out', 'sub', 'inner.txt'), 'utf8'), 'inner\n');
    assert.equal(readFileSync(join(dir, '.complete'), 'utf8'), 'v1.2.3\n');
    // The package file is gone once unpacked; the earlier release and the staging folder are gone.
    assert.deepEqual(readdirSync(dir).sort(), ['.complete', 'out']);
    assert.deepEqual(readdirSync(cache), ['v1.2.3']);
    assert.deepEqual(github.requests(), [
      `/repos/${REPO}/releases/latest`,
      `/repos/${REPO}/compare/main...v1.2.3`,
      'asset:manifest.json',
      'asset:thing-1.2.3.tgz',
    ]);
    // unpack(file, part, pkg): the downloaded file sits in the staging folder, pkg is the manifest entry.
    assert.equal(calls.length, 1);
    assert.equal(calls[0].name, 'thing-1.2.3.tgz');
    assert.equal(calls[0].part, `${dir}.part`);
    assert.equal(calls[0].dir, calls[0].part);
    assert.equal(calls[0].pkg.file, 'thing-1.2.3.tgz');
    assert.match(calls[0].pkg.sha256, /^[0-9a-f]{64}$/);
    assert.deepEqual([...log], [`thing: v1.2.3 downloaded from ${REPO}`]);
  } finally {
    await github.close();
  }
});

test('a release already in the cache is not downloaded again', async () => {
  const github = await fakeGitHub({ repo: REPO, tag: 'v1.2.3', packages: [thingPackage()] });
  const cache = fresh('cache');
  const { calls, unpack } = unpacker();
  const log = lines();
  try {
    const first = await latestMainRelease({ repo: REPO, cache, label: 'thing', unpack, log: log.log, api: github.api });
    const before = github.requests().length;
    const second = await latestMainRelease({ repo: REPO, cache, label: 'thing', unpack, log: log.log, api: github.api });
    assert.deepEqual(second, first);
    assert.deepEqual(github.requests().slice(before), [`/repos/${REPO}/releases/latest`, `/repos/${REPO}/compare/main...v1.2.3`]);
    assert.equal(calls.length, 1);
    assert.deepEqual([...log], [`thing: v1.2.3 downloaded from ${REPO}`]);
  } finally {
    await github.close();
  }
});

test('a tag that is not on main is not used', async () => {
  const github = await fakeGitHub({ repo: REPO, tag: 'v1.2.3', status: 'diverged', packages: [thingPackage()] });
  const cache = fresh('cache');
  mkdirSync(join(cache, 'v1.0.0'), { recursive: true });
  const log = lines();
  const { calls, unpack } = unpacker();
  try {
    const found = await latestMainRelease({ repo: REPO, cache, label: 'thing', unpack, log: log.log, api: github.api });
    assert.equal(found, null);
    assert.deepEqual([...log], ['thing: v1.2.3 is not on main (diverged), not used']);
    // Nothing was downloaded and the cache is as it was.
    assert.deepEqual(github.requests(), [`/repos/${REPO}/releases/latest`, `/repos/${REPO}/compare/main...v1.2.3`]);
    assert.equal(calls.length, 0);
    assert.deepEqual(readdirSync(cache), ['v1.0.0']);
  } finally {
    await github.close();
  }
});

test('a tag that is ahead of main is not used either', async () => {
  const github = await fakeGitHub({ repo: REPO, tag: 'v1.2.3', status: 'ahead', packages: [thingPackage()] });
  const log = lines();
  try {
    const found = await latestMainRelease({ repo: REPO, cache: fresh('cache'), label: 'thing', unpack: () => {}, log: log.log, api: github.api });
    assert.equal(found, null);
    assert.deepEqual([...log], ['thing: v1.2.3 is not on main (ahead), not used']);
  } finally {
    await github.close();
  }
});

test('a package whose SHA-256 differs leaves no copy', async () => {
  const github = await fakeGitHub({
    repo: REPO,
    tag: 'v1.2.3',
    packages: [pack('thing-1.2.3.tgz', { 'hello.txt': 'hello\n' }, { sha256: '0'.repeat(64) })],
  });
  const cache = fresh('cache');
  mkdirSync(join(cache, 'v1.0.0'), { recursive: true });
  const log = lines();
  const { calls, unpack } = unpacker();
  try {
    const found = await latestMainRelease({ repo: REPO, cache, label: 'thing', unpack, log: log.log, api: github.api });
    assert.equal(found, null);
    assert.deepEqual([...log], ['thing: v1.2.3 could not be downloaded (thing-1.2.3.tgz: SHA-256 differs from the manifest)']);
    assert.equal(calls.length, 0, 'a package that fails its check is never unpacked');
    // No <tag> folder, no <tag>.part; the earlier release stays, as the cache is only cleaned after a good download.
    assert.equal(existsSync(join(cache, 'v1.2.3')), false);
    assert.equal(existsSync(join(cache, 'v1.2.3.part')), false);
    assert.deepEqual(readdirSync(cache), ['v1.0.0']);
  } finally {
    await github.close();
  }
});

test('a package that cannot be unpacked leaves no copy', async () => {
  const github = await fakeGitHub({ repo: REPO, tag: 'v1.2.3', packages: [thingPackage()] });
  const cache = fresh('cache');
  const log = lines();
  try {
    const found = await latestMainRelease({
      repo: REPO, cache, label: 'thing', log: log.log, api: github.api,
      unpack: () => { throw new Error('cannot unpack'); },
    });
    assert.equal(found, null);
    assert.deepEqual([...log], ['thing: v1.2.3 could not be downloaded (cannot unpack)']);
    assert.deepEqual(readdirSync(cache), []);
  } finally {
    await github.close();
  }
});

test('no network keeps the cache and returns null', async () => {
  const api = await closedApi();
  const cache = fresh('cache');
  mkdirSync(join(cache, 'v1.0.0'), { recursive: true });
  writeFileSync(join(cache, 'v1.0.0', '.complete'), 'v1.0.0\n');
  const log = lines();
  const found = await latestMainRelease({ repo: REPO, cache, label: 'thing', unpack: () => {}, log: log.log, api });
  assert.equal(found, null);
  assert.equal(log.length, 1);
  assert.match(log[0], /^thing: latest release of acme\/thing not reachable \(.+\)$/);
  assert.deepEqual(readdirSync(cache), ['v1.0.0']);
  assert.equal(readFileSync(join(cache, 'v1.0.0', '.complete'), 'utf8'), 'v1.0.0\n');
});

test('a cache path with spaces unpacks', async () => {
  const github = await fakeGitHub({ repo: REPO, tag: 'v1.2.3', packages: [thingPackage()] });
  const cache = join(fresh('cache with spaces'), 'my cache');
  const log = lines();
  const { unpack } = unpacker();
  try {
    const found = await latestMainRelease({ repo: REPO, cache, label: 'thing', unpack, log: log.log, api: github.api });
    assert.deepEqual(found, { tag: 'v1.2.3', dir: join(cache, 'v1.2.3') });
    assert.equal(readFileSync(join(cache, 'v1.2.3', 'out', 'sub', 'inner.txt'), 'utf8'), 'inner\n');
    assert.deepEqual([...log], [`thing: v1.2.3 downloaded from ${REPO}`]);
  } finally {
    await github.close();
  }
});

test('GH_TOKEN and GITHUB_TOKEN only lift the rate limit: sent when set, never needed', async () => {
  const saved = { GH_TOKEN: process.env.GH_TOKEN, GITHUB_TOKEN: process.env.GITHUB_TOKEN };
  const github = await fakeGitHub({ repo: REPO, tag: 'v1.2.3', packages: [thingPackage()] });
  try {
    delete process.env.GH_TOKEN;
    delete process.env.GITHUB_TOKEN;
    const plain = await latestMainRelease({ repo: REPO, cache: fresh('cache'), label: 'thing', unpack: unpacker().unpack, log: () => {}, api: github.api });
    assert.equal(plain?.tag, 'v1.2.3');
    assert.ok(github.authorizations().length >= 4);
    assert.ok(github.authorizations().every((value) => value === undefined), 'no token, no Authorization header');
    const before = github.authorizations().length;
    process.env.GITHUB_TOKEN = 'secret-token';
    await latestMainRelease({ repo: REPO, cache: fresh('cache'), label: 'thing', unpack: unpacker().unpack, log: () => {}, api: github.api });
    assert.ok(github.authorizations().slice(before).every((value) => value === 'Bearer secret-token'));
  } finally {
    for (const [name, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
    await github.close();
  }
});

test('untar unpacks a release package without its top folder, creating the target', () => {
  const file = join(fresh('one'), 'one.tgz');
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, tgz({ 'a.txt': 'a\n', 'd/b.txt': 'b\n' }));
  const into = join(fresh('into with spaces'), 'deep', 'er');
  untar(file, into);
  assert.deepEqual(readdirSync(into).sort(), ['a.txt', 'd']);
  assert.equal(readFileSync(join(into, 'a.txt'), 'utf8'), 'a\n');
  assert.equal(readFileSync(join(into, 'd', 'b.txt'), 'utf8'), 'b\n');
});

// ArtCraft: the four engines of artcraft-web's release, each unpacked into <tag>/<engine>/ without the npm
// package's own package.json and README.md. The log lines are the ones sync-artcraft has always written.
const engineFiles = (engine) => ({
  [`${engine}.js`]: `// ${engine}\n`,
  [`${engine}_bg.wasm`]: `wasm ${engine}\n`,
  'provenance.json': JSON.stringify({ tag: 'v0.1.0', commit: 'abc1234'.padEnd(40, '0'), files: {} }),
  'package.json': '{ "name": "@artcraft-web/engine" }\n',
  'README.md': '# engine\n',
});
const enginePackage = (engine, extra = {}) => pack(`artcraft-web-${engine}-0.4.0.tgz`, engineFiles(engine), { engine, ...extra });

test('artcraft keeps its messages', async (t) => {
  const ARTCRAFT_LOG = (tag) => `artcraft-web: ${tag} downloaded from ${ARTCRAFT_REPO}`;
  // The manifest of the real release lists the engines in alphabetical order, not in ARTCRAFT_ENGINES order.
  const all = ['deckcraft', 'gridcraft', 'pdfcraft', 'wordcraft'].map((engine) => enginePackage(engine));

  await t.test('a good release: <tag>/<engine>/ without package.json and README.md', async () => {
    const github = await fakeGitHub({ repo: ARTCRAFT_REPO, tag: 'v0.4.0', packages: all });
    const cache = fresh('artcraft-cache');
    mkdirSync(join(cache, 'v0.3.0'), { recursive: true });
    const log = lines();
    try {
      const found = await latestArtcraftRelease(cache, log.log, github.api);
      const dir = join(cache, 'v0.4.0');
      assert.deepEqual(found, { tag: 'v0.4.0', dir });
      assert.deepEqual([...ARTCRAFT_ENGINES].sort(), readdirSync(dir).filter((name) => name !== '.complete').sort());
      for (const engine of ARTCRAFT_ENGINES) {
        assert.deepEqual(readdirSync(join(dir, engine)).sort(), ['provenance.json', `${engine}.js`, `${engine}_bg.wasm`].sort());
      }
      assert.equal(readFileSync(join(dir, '.complete'), 'utf8'), 'v0.4.0\n');
      assert.deepEqual(readdirSync(cache), ['v0.4.0']);
      assert.deepEqual([...log], [ARTCRAFT_LOG('v0.4.0')]);
    } finally {
      await github.close();
    }
  });

  await t.test('a tag that is not on main', async () => {
    const github = await fakeGitHub({ repo: ARTCRAFT_REPO, tag: 'v0.4.0', status: 'diverged', packages: all });
    const log = lines();
    try {
      assert.equal(await latestArtcraftRelease(fresh('artcraft-cache'), log.log, github.api), null);
      assert.deepEqual([...log], ['artcraft-web: v0.4.0 is not on main (diverged), not used']);
    } finally {
      await github.close();
    }
  });

  await t.test('a package that differs from the manifest', async () => {
    const packages = all.map((p) => (p.extra.engine === 'gridcraft' ? { ...p, extra: { ...p.extra, sha256: '0'.repeat(64) } } : p));
    const github = await fakeGitHub({ repo: ARTCRAFT_REPO, tag: 'v0.4.0', packages });
    const cache = fresh('artcraft-cache');
    const log = lines();
    try {
      assert.equal(await latestArtcraftRelease(cache, log.log, github.api), null);
      assert.deepEqual([...log], ['artcraft-web: v0.4.0 could not be downloaded (artcraft-web-gridcraft-0.4.0.tgz: SHA-256 differs from the manifest)']);
      assert.deepEqual(readdirSync(cache), [], 'no <tag> and no <tag>.part');
    } finally {
      await github.close();
    }
  });

  await t.test('an engine missing from the manifest', async () => {
    const github = await fakeGitHub({ repo: ARTCRAFT_REPO, tag: 'v0.4.0', packages: all.filter((p) => p.extra.engine !== 'deckcraft') });
    const cache = fresh('artcraft-cache');
    const log = lines();
    try {
      assert.equal(await latestArtcraftRelease(cache, log.log, github.api), null);
      assert.deepEqual([...log], ['artcraft-web: v0.4.0 could not be downloaded (deckcraft is not in the manifest of v0.4.0)']);
      assert.deepEqual(readdirSync(cache), []);
    } finally {
      await github.close();
    }
  });

  await t.test('no network', async () => {
    const log = lines();
    assert.equal(await latestArtcraftRelease(fresh('artcraft-cache'), log.log, await closedApi()), null);
    assert.equal(log.length, 1);
    assert.match(log[0], /^artcraft-web: latest release of acaprino\/artcraft-web not reachable \(.+\)$/);
  });
});
