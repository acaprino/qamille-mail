// Every browser test of the preview in one command, one at a time, against one Vite server of its
// own on a free port. Tests that start their own server keep doing so. Synthetic data only.
//
//   npm run test:browser                run them all
//   npm run test:browser -- smoke undo  run those
//
// The *-native tests need the built app and are not part of this: see their own npm scripts.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';

const desktop = path.resolve(import.meta.dirname, '..');
const tests = path.join(desktop, 'tests');

// These are written for node --test (activity also drives the preview); the rest are plain scripts.
const NODE_TEST = new Set([
  'activity', 'assistant-server', 'backend-text', 'calendar-gate', 'close-guards', 'calendar-series', 'conversation-rows', 'events', 'github-release',
  'meetings-follow', 'meetings-highlight', 'meetings-preview', 'meetings-view', 'people', 'preview-search', 'regional', 'reply-all', 'shortcut-guard', 'thread-tree',
  'sigrid-sync', 'sigrid-wiring', 'sigrid-words', 'theme-css', 'undo-notices', 'undo-close', 'mailto', 'row-menu', 'notification-open',
]);
// Tools and checks that are not browser tests of the preview, or take arguments.
// glance-scale also drives the built executable; ds-export runs two full exports (minutes each);
// build-check makes a production build of its own (node tests/build-check.mjs).
// They are run by themselves, not here.
const OTHERS = new Set([
  'capture', 'style-tokens', 'ds-usage', 'visual-snapshot', 'visual-compare', 'release-theme-probe',
  'glance-scale', 'ds-export', 'tokens-compatto', 'contrast', 'build-check',
]);

const all = fs
  .readdirSync(tests)
  .filter((name) => name.endsWith('.mjs') && !name.startsWith('_tmp_'))
  .map((name) => name.slice(0, -'.mjs'.length))
  .filter((name) => !name.endsWith('-native') && !OTHERS.has(name))
  .sort();
const wanted = process.argv.slice(2);
const unknown = wanted.filter((name) => !all.includes(name));
if (unknown.length) {
  console.error(`Unknown test: ${unknown.join(', ')}\nAvailable: ${all.join(', ')}`);
  process.exit(2);
}
const names = wanted.length ? wanted : all;

const freePort = () =>
  new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.once('error', reject);
    probe.listen(0, '127.0.0.1', () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });

const port = await freePort();
const url = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: desktop,
  windowsHide: true,
  stdio: 'pipe',
});
let serverOutput = '';
server.stdout.on('data', (data) => (serverOutput += data));
server.stderr.on('data', (data) => (serverOutput += data));

const failures = [];
try {
  for (let attempt = 0; ; attempt++) {
    if (server.exitCode !== null) throw new Error(`Vite stopped: ${serverOutput}`);
    try {
      if ((await fetch(url)).ok) break;
    } catch {
      // Still starting.
    }
    if (attempt === 100) throw new Error(`Vite did not start: ${serverOutput}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  for (const name of names) {
    const started = Date.now();
    const args = NODE_TEST.has(name) ? ['--test', `tests/${name}.mjs`] : [`tests/${name}.mjs`];
    const run = spawnSync(process.execPath, args, {
      cwd: desktop,
      encoding: 'utf8',
      timeout: 300_000,
      maxBuffer: 64 * 1024 * 1024,
      env: { ...process.env, QAMILLE_PREVIEW_URL: url },
    });
    const text = `${run.stdout ?? ''}${run.stderr ?? ''}`;
    const seconds = Math.round((Date.now() - started) / 1000);
    const passed = run.status === 0;
    const timedOut = run.error?.code === 'ETIMEDOUT';
    console.log(`${passed ? 'ok  ' : 'FAIL'} ${name} (${seconds}s)${timedOut ? ' TIMED OUT after 300 s' : ''}`);
    if (!passed) {
      failures.push(name);
      console.log(text.split('\n').filter((line) => line.trim()).slice(-25).join('\n'));
    }
  }
} finally {
  server.kill();
}
console.log(`\n${names.length - failures.length}/${names.length} passed${failures.length ? `; failed: ${failures.join(', ')}` : ''}`);
process.exit(failures.length ? 1 : 0);
