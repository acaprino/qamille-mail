// Session end (audit F2) and the app zoom keys (F9), without Tauri: the close guards' session steps
// (src/lib/close-guards.ts) and zoomKey (src/lib/shortcut-guard.ts). Run by node --test.
// Node strips the type annotations of these dependency-free modules.
import assert from 'node:assert/strict';
import path from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

const lib = (name) => pathToFileURL(path.resolve(import.meta.dirname, '../src/lib', name)).href;
const { createCloseGuards } = await import(lib('close-guards.ts'));
const { zoomKey } = await import(lib('shortcut-guard.ts'));

test('session end runs every step, in the close order, with no veto and no dialog', async () => {
  const guards = createCloseGuards();
  const log = [];
  // The meetings guard is registered first but `last`: it stops the recording after the others saved.
  guards.register('meetings', async () => (log.push('ask meetings'), false), {
    last: true,
    sessionEnd: async (last) => log.push(`meetings ${last ? 'stop' : 'notes'}`),
  });
  guards.register('composers', async () => (log.push('ask composers'), false), {
    sessionEnd: async (last) => log.push(`composers ${last}`),
  });
  guards.register('settings', async () => false, {
    sessionEnd: async () => {
      log.push('settings');
      throw new Error('disk full');
    },
  });
  guards.register('no-step', async () => true);

  // The settings step fails: the others still run, and the caller learns that not everything was
  // saved ("Ricarica la finestra" then asks before reloading, audit T1).
  assert.equal(await guards.endSession(false), false);
  assert.equal(await guards.endSession(true), false);
  assert.deepEqual(log, [
    'composers false',
    'settings',
    'meetings notes',
    'composers true',
    'settings',
    'meetings stop',
  ]);
});

test('a step that answers false counts as not saved', async () => {
  const guards = createCloseGuards();
  let ran = 0;
  // A save that fails without throwing (a draft, the settings, the meeting notes) answers false.
  guards.register('drafts', async () => true, { sessionEnd: async () => (ran++, false) });
  guards.register('settings', async () => true, { sessionEnd: async () => (ran++, true) });
  assert.equal(await guards.endSession(false), false);
  // The step after the failed one still ran.
  assert.equal(ran, 2);
});

test('a step that answers nothing or true counts as saved', async () => {
  const guards = createCloseGuards();
  guards.register('drafts', async () => true, { sessionEnd: async () => {} });
  guards.register('settings', async () => true, { sessionEnd: async () => true });
  assert.equal(await guards.endSession(false), true);
  assert.equal(await guards.endSession(true), true);
});

test('an unregistered guard takes its session step with it', async () => {
  const guards = createCloseGuards();
  let ran = 0;
  const stop = guards.register('composers', async () => true, { sessionEnd: async () => ran++ });
  stop();
  assert.equal(await guards.endSession(true), true);
  assert.equal(ran, 0);
});

test('the close guards still veto as before', async () => {
  const guards = createCloseGuards();
  guards.register('settings', async () => false, { sessionEnd: async () => {} });
  assert.equal(await guards.run(), false);
});

test('zoom keys: Ctrl with = + - 0, main row and keypad; nothing else', () => {
  const key = (key, extra = {}) => zoomKey({ key, code: '', ctrlKey: true, metaKey: false, altKey: false, ...extra });
  assert.equal(key('='), 'in');
  assert.equal(key('+'), 'in');
  assert.equal(key('+', { code: 'NumpadAdd' }), 'in');
  assert.equal(key('-'), 'out');
  assert.equal(key('-', { code: 'NumpadSubtract' }), 'out');
  assert.equal(key('0'), 'reset');
  assert.equal(key(')', { code: 'Digit0' }), 'reset');
  assert.equal(key('0', { code: 'Numpad0' }), 'reset');
  // Num Lock off: the keypad 0 is Insert, and Ctrl+Ins copies.
  assert.equal(key('Insert', { code: 'Numpad0' }), null);
  assert.equal(key('='), 'in');
  assert.equal(key('k'), null);
  assert.equal(key('=', { ctrlKey: false }), null);
  // AltGr is Ctrl+Alt: typing a character is not zooming.
  assert.equal(key('+', { altKey: true }), null);
});
