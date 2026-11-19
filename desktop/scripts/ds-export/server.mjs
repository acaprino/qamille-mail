// The Vite server the captures run against, and the build that gives bundle.css its compiled rules.
// It never uses the port of `npm run dev` (1420): it asks for PORT with --strictPort, or for a free one.
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { desktop } from './config.mjs';

const vite = path.join(desktop, 'node_modules/vite/bin/vite.js');

function listens(port) {
  return new Promise((resolve) => {
    const probe = net.createServer();
    probe.once('error', () => resolve(false));
    probe.listen(port, '127.0.0.1', () => probe.close(() => resolve(true)));
  });
}

/** `preferred` when it is free, otherwise a port the system hands out. Never 1420. */
export async function pickPort(preferred) {
  if (preferred === 1420) throw new Error('port 1420 belongs to `npm run dev`');
  if (await listens(preferred)) return preferred;
  return await new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.once('error', reject);
    probe.listen(0, '127.0.0.1', () => { const { port } = probe.address(); probe.close(() => resolve(port)); });
  });
}

function run(args, label) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [vite, ...args], { cwd: desktop, windowsHide: true, stdio: 'pipe' });
    let output = '';
    child.stdout.on('data', (d) => (output += d));
    child.stderr.on('data', (d) => (output += d));
    child.on('exit', (code) => (code === 0 ? resolve(output) : reject(new Error(`${label} failed (${code}):\n${output.slice(-2000)}`))));
  });
}

/** `vite build` of the app into `outDir`. The engines under src/vendor come from `npm run artcraft:sync`. */
export async function build(outDir) {
  if (!fs.existsSync(path.join(desktop, 'src/vendor/artcraft'))) {
    throw new Error('src/vendor/artcraft is missing: run `npm run artcraft:sync` in desktop/ first');
  }
  await run(['build', '--outDir', outDir, '--emptyOutDir'], 'vite build');
}

/** The dev server, up and answering. `stop()` ends it. */
export async function startDev(port) {
  const child = spawn(process.execPath, [vite, '--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: desktop, windowsHide: true, stdio: 'pipe' });
  let output = '';
  child.stdout.on('data', (d) => (output += d));
  child.stderr.on('data', (d) => (output += d));
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 150; i++) {
    if (child.exitCode != null) throw new Error(`the Vite server stopped:\n${output.slice(-2000)}`);
    try { if ((await fetch(base)).ok) return { base, stop: () => child.kill() }; } catch { /* starting */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  child.kill();
  throw new Error(`the Vite server did not start on ${port}:\n${output.slice(-2000)}`);
}
