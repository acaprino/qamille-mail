// The guided form of access wave 2 (spec A8), as pure transitions: src/lib/server-connect.ts.
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  canLeaveServer,
  cancelled,
  consentGiven,
  deviceDisabled,
  leaveServer,
  localSuggestion,
  LOCAL_SERVER_URL,
  normalizeUrl,
  probed,
  signedIn,
  start,
  typeUrl,
} from '../src/lib/server-connect.ts';

const found = { kind: 'found', info: { version: '1.0', api: 1, signup: false, password_reset: false, edition: 'self-hosted' } };

test('«Next» needs a server found at the address typed now', () => {
  let form = start({ consented: [] });
  assert.equal(form.step, 'server');
  form = typeUrl(form, 'https://calcolo.ufficio.it');
  assert.equal(canLeaveServer(form), false, 'no answer yet');
  form = typeUrl(form, 'https://altro.it');
  form = probed(form, 'https://calcolo.ufficio.it', found);
  assert.equal(canLeaveServer(form), false, 'an answer for an older address is dropped');
  form = probed(form, 'https://altro.it', { kind: 'unreachable' });
  assert.equal(canLeaveServer(form), false);
  form = probed(form, 'https://altro.it', found);
  assert.equal(canLeaveServer(form), true);
  assert.equal(leaveServer(form).step, 'consent');
});

test('consent is asked once per address', () => {
  let form = start({ consented: ['https://calcolo.ufficio.it'] });
  form = probed(typeUrl(form, 'HTTPS://Calcolo.Ufficio.it/'), 'HTTPS://Calcolo.Ufficio.it/', found);
  assert.equal(leaveServer(form).step, 'sign_in', 'already agreed for this address');
  form = probed(typeUrl(form, 'https://nuovo.it'), 'https://nuovo.it', found);
  form = consentGiven(leaveServer(form));
  assert.equal(form.step, 'sign_in');
  assert.deepEqual(form.consented, ['https://calcolo.ufficio.it', 'https://nuovo.it']);
  assert.equal(normalizeUrl(' HTTPS://A.Example/ '), 'https://a.example');
});

test('the Professional form starts from consent with its own address', () => {
  const form = start({ preset: 'https://qamille.example', consented: [] });
  assert.equal(form.step, 'consent');
  assert.equal(typeUrl(form, 'https://altro.it').url, 'https://qamille.example', 'the address cannot change');
  assert.equal(start({ preset: 'https://qamille.example', consented: ['https://qamille.example'] }).step, 'sign_in');
});

test('sign-in outcomes, and no password once done or cancelled', () => {
  let form = { ...start({ preset: 'https://s.example', consented: ['https://s.example'] }), identifier: 'mario', password: 'segreta lunga' };
  form = signedIn(form, { kind: 'password_change_required' });
  assert.equal(form.step, 'new_password');
  const devices = [{ id: 'a', name: 'Vecchio', created_at: '', last_seen_at: null, current: false }];
  form = signedIn({ ...form, step: 'sign_in' }, { kind: 'device_limit', devices });
  assert.equal(form.step, 'device_limit');
  form = deviceDisabled(form, 'a');
  assert.equal(form.step, 'sign_in');
  assert.equal(form.password, 'segreta lunga', 'the retry keeps the credentials');
  const done = signedIn(form, { kind: 'connected', status: {} });
  assert.equal(done.step, 'done');
  assert.equal(done.password, '');
  assert.equal(cancelled(form).password, '');
});

test('«Sign in again» opens on «Sign in» for the session address (EX-15)', () => {
  const form = start({ consented: ['https://calcolo.ufficio.it'], reconnect: 'https://calcolo.ufficio.it' });
  assert.equal(form.step, 'sign_in');
  assert.equal(form.url, 'https://calcolo.ufficio.it');
  assert.equal(start({ consented: [], reconnect: 'https://calcolo.ufficio.it' }).step, 'consent', 'consent withdrawn: asked again');
});

test('the local suggestion appears only on an empty server step without a preset, when the local probe found a server', () => {
  const empty = start({ consented: [] });
  assert.equal(LOCAL_SERVER_URL, 'http://127.0.0.1:8787');
  assert.equal(localSuggestion(empty, found), true);
  assert.equal(localSuggestion(empty, null), false, 'no local probe answer yet');
  assert.equal(localSuggestion(empty, { kind: 'unreachable' }), false);
  assert.equal(localSuggestion(empty, { kind: 'not_qamille' }), false);
  assert.equal(localSuggestion(start({ preset: 'https://qamille.example' }), found), false, 'the Professional build has a preset');
  assert.equal(localSuggestion(leaveServer(probed(typeUrl(empty, 'https://a.it'), 'https://a.it', found)), found), false, 'only on the Server step');
});

test('a form with a typed address never shows the local suggestion', () => {
  const typed = typeUrl(start({ consented: [] }), 'https://calcolo.ufficio.it');
  assert.equal(localSuggestion(typed, found), false);
  assert.equal(localSuggestion(typeUrl(typed, '  '), found), true, 'blank again: it is offered again');
});
