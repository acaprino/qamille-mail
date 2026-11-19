// Tests of release.mjs on a sandbox copy of the files named in the real .github/release.json: the version files are
// never touched in the repository. Run: node --test .github/scripts/release.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..', '..');
const script = join(here, 'release.mjs');
const realConfig = JSON.parse(readFileSync(join(root, '.github', 'release.json'), 'utf8'));

function release(cwd, ...args) {
  const run = spawnSync(process.execPath, [script, ...args], { cwd, encoding: 'utf8' });
  return { code: run.status, out: run.stdout.trim(), err: run.stderr.trim() };
}

function sandbox(config, files) {
  const dir = mkdtempSync(join(tmpdir(), 'release-test-'));
  mkdirSync(join(dir, '.github'), { recursive: true });
  writeFileSync(join(dir, '.github', 'release.json'), JSON.stringify(config));
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), text);
  }
  return dir;
}

const cleanup = (dir) => rmSync(dir, { recursive: true, force: true });
const manifest = (name, version) => `[package]\nname = "${name}"\nversion = "${version}"\nedition = "2024"\n\n[dependencies]\nserde = "1"\n`;
const NL = String.fromCharCode(10);
const lockBlock = ([name, version, source]) => ['[[package]]', `name = "${name}"`, `version = "${version}"`, ...(source ? [`source = "${source}"`] : []), ''].join(NL);
const lock = (...crates) => ['version = 4', '', crates.map(lockBlock).join(NL)].join(NL);
const escape = (version) => version.replace(/\./g, '\\.');

test('real configuration: server, compute and the app library follow the app, with their locks and the core tag', () => {
  assert.ok(realConfig.files.includes('desktop/src-tauri/Cargo.toml'));
  for (const path of ['server/Cargo.toml', 'compute/Cargo.toml', 'app/Cargo.toml']) {
    assert.ok(realConfig.adopt.includes(path), `${path} is an adopted version file`);
  }
  // the public repository has no enterprise folder (its release.json is the overlay without that lock)
  const hasEnterprise = existsSync(join(root, 'enterprise', 'server', 'Cargo.lock'));
  for (const path of ['server/Cargo.lock', 'compute/Cargo.lock', 'app/Cargo.lock', ...(hasEnterprise ? ['enterprise/server/Cargo.lock'] : [])]) {
    assert.ok(realConfig.locks.includes(path), `${path} is a bumped lock`);
  }
  assert.deepEqual([...realConfig.adoptCrates].sort(), ['qamille', 'qamille-compute', 'qamille-server']);
  assert.deepEqual(realConfig.extraTags, ['core-v']);
});

test('real configuration: a bump on a copy aligns every version file, lock and crate, and the repository is untouched', () => {
  const dir = mkdtempSync(join(tmpdir(), 'release-real-'));
  try {
    const paths = [...realConfig.files, ...realConfig.adopt, ...realConfig.locks];
    for (const path of ['.github/release.json', ...paths]) {
      mkdirSync(dirname(join(dir, path)), { recursive: true });
      cpSync(join(root, path), join(dir, path));
    }
    const before = release(dir, 'current').out;
    const next = release(dir, 'next', 'patch').out;
    const bumped = release(dir, 'bump', 'patch');
    assert.equal(bumped.code, 0, bumped.err);
    assert.equal(bumped.out, next);
    assert.notEqual(next, before);
    for (const path of [...realConfig.adopt, 'desktop/src-tauri/Cargo.toml']) {
      assert.match(readFileSync(join(dir, path), 'utf8'), new RegExp(`^version = "${escape(next)}"\\r?$`, 'm'), path);
    }
    for (const path of realConfig.locks) {
      const blocks = readFileSync(join(dir, path), 'utf8').replace(/\r\n/g, '\n').split('\n[[package]]\n');
      for (const name of [...realConfig.crates, ...realConfig.adoptCrates]) {
        const block = blocks.find((b) => new RegExp(`^name = "${name}"$`, 'm').test(b) && !/^source = /m.test(b));
        if (block) assert.match(block, new RegExp(`^version = "${escape(next)}"$`, 'm'), `${path}: ${name}`);
      }
    }
    // the enterprise crate keeps its own version: it is neither in crates nor in adoptCrates
    if (realConfig.locks.includes('enterprise/server/Cargo.lock')) {
      assert.match(readFileSync(join(dir, 'enterprise/server/Cargo.lock'), 'utf8'), /name = "qamille-server-enterprise"\r?\nversion = "0\.1\.0"/);
    }
    assert.deepEqual(release(dir, 'tags').out.split('\n'), [`v${next}`, `core-v${next}`]);
    assert.equal(release(root, 'current').out, before, 'the repository still has its version');
  } finally { cleanup(dir); }
});

test('tags: the release tag first, then every extra prefix, for the current or a given version', () => {
  const dir = sandbox({ app: 'X', tagPrefix: 'v', extraTags: ['core-v'], files: ['a/Cargo.toml'] }, { 'a/Cargo.toml': manifest('a', '1.2.3') });
  try {
    assert.deepEqual(release(dir, 'tags').out.split('\n'), ['v1.2.3', 'core-v1.2.3']);
    assert.deepEqual(release(dir, 'tags', '9.9.9').out.split('\n'), ['v9.9.9', 'core-v9.9.9']);
  } finally { cleanup(dir); }
});

test('tags: without extraTags only the release tag', () => {
  const dir = sandbox({ app: 'X', tagPrefix: 'v', files: ['a/Cargo.toml'] }, { 'a/Cargo.toml': manifest('a', '1.2.3') });
  try { assert.equal(release(dir, 'tags').out, 'v1.2.3'); } finally { cleanup(dir); }
});

test('adopt: a file behind the app joins at the next version; a file in `files` that is behind still stops the bump', () => {
  const config = { app: 'X', tagPrefix: 'v', files: ['app/Cargo.toml'], adopt: ['core/Cargo.toml'], locks: ['core/Cargo.lock'], crates: ['app'], adoptCrates: ['core'] };
  const files = {
    'app/Cargo.toml': manifest('app', '0.2.0'),
    'core/Cargo.toml': manifest('core', '0.1.0'),
    'core/Cargo.lock': lock(['core', '0.1.0'], ['serde', '1.0.0', 'registry+https://github.com/rust-lang/crates.io-index']),
  };
  const dir = sandbox(config, files);
  try {
    assert.equal(release(dir, 'bump', 'minor').out, '0.3.0');
    assert.match(readFileSync(join(dir, 'core/Cargo.toml'), 'utf8'), /^version = "0\.3\.0"$/m);
    const text = readFileSync(join(dir, 'core/Cargo.lock'), 'utf8');
    assert.match(text, /name = "core"\nversion = "0\.3\.0"/);
    assert.match(text, /name = "serde"\nversion = "1\.0\.0"/, 'a registry crate is never rewritten');
    assert.deepEqual(release(dir, 'paths').out.split('\n'), ['app/Cargo.toml', 'core/Cargo.toml', 'core/Cargo.lock']);
  } finally { cleanup(dir); }

  const strict = sandbox({ ...config, files: ['app/Cargo.toml', 'core/Cargo.toml'], adopt: [] }, files);
  try {
    const run = release(strict, 'bump', 'patch');
    assert.equal(run.code, 1);
    assert.match(run.err, /align them first/);
  } finally { cleanup(strict); }
});

test('adopt: a lock without any listed crate still fails the bump', () => {
  const dir = sandbox(
    { app: 'X', tagPrefix: 'v', files: ['app/Cargo.toml'], locks: ['other/Cargo.lock'], crates: ['app'], adoptCrates: ['core'] },
    { 'app/Cargo.toml': manifest('app', '1.0.0'), 'other/Cargo.lock': lock(['unrelated', '1.0.0']) },
  );
  try {
    const run = release(dir, 'bump', 'patch');
    assert.equal(run.code, 1);
    assert.match(run.err, /none of app is at 1\.0\.0/);
  } finally { cleanup(dir); }
});

test('adopt keeps CRLF line endings', () => {
  const dir = sandbox({ app: 'X', tagPrefix: 'v', files: ['app/Cargo.toml'], adopt: ['core/Cargo.toml'] }, {
    'app/Cargo.toml': manifest('app', '1.0.0'),
    'core/Cargo.toml': manifest('core', '0.1.0').replace(/\n/g, '\r\n'),
  });
  try {
    assert.equal(release(dir, 'bump', 'patch').out, '1.0.1');
    const text = readFileSync(join(dir, 'core/Cargo.toml'), 'utf8');
    assert.match(text, /version = "1\.0\.1"\r\n/);
    assert.doesNotMatch(text, /[^\r]\n/);
  } finally { cleanup(dir); }
});

test('the release workflow tags and pushes every tag of `release.mjs tags`', () => {
  const workflow = readFileSync(join(root, '.github', 'workflows', 'release.yml'), 'utf8');
  assert.match(workflow, /release\.mjs tags \$version/);
  assert.match(workflow, /if \(git tag --list "\$\(node \.github\/scripts\/release\.mjs config tagPrefix\)\*"\) \{\s+\$version = node \.github\/scripts\/release\.mjs bump /, 'the first release keeps the current version');
  assert.match(workflow, /git commit --allow-empty /, 'the first release has nothing to commit');
  assert.match(workflow, /foreach \(\$t in \$tags\) \{ git tag -a \$t/);
  assert.match(workflow, /refs\/tags\/\$_/);
  assert.match(workflow, /gh release create \$tag /, 'the GitHub Release is for the first tag only');
});
