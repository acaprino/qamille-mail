// The mailbox synchronization (wave 7): consent line, status texts, removal notices, revocation
// results and the choice in «Dispositivi». Pure functions of `src/lib/mailbox-sync.ts`.
import assert from 'node:assert/strict';
import test from 'node:test';
import { backendTextItalian } from '../src/lib/backend-text.ts';
import { parseAppEvent } from '../src/lib/events.ts';
import {
  GOOGLE_PERMISSIONS_URL,
  canTurnOn,
  consentLine,
  mailboxesChanged,
  removalText,
  retryable,
  revokeChoice,
  revokeText,
  statusText,
} from '../src/lib/mailbox-sync.ts';

const status = (over = {}) => ({ mode: 'idle', enabled: true, consented: true, last_sync: null, error: null, removals: [], mailboxes: [], ...over });

test('the consent line appears only when the server declares sync', () => {
  assert.equal(
    consentLine({ sync: { available: true } }).it,
    'Le caselle che colleghi, con le loro credenziali, vengono salvate su questo server per ritrovarle sugli altri tuoi computer. Chi gestisce il server può accedervi.',
  );
  assert.match(consentLine({ sync: { available: true } }).en, /credentials, are saved on this server/);
  for (const info of [null, undefined, {}, { sync: null }, { sync: { available: false } }]) assert.equal(consentLine(info), null);
});

test('the status text says what is going on', () => {
  const explain = 'La sincronizzazione passa da un server Qamille.';
  assert.equal(statusText(null).it, explain);
  assert.equal(statusText(status({ mode: 'no_server' })).it, explain);
  assert.equal(statusText(status({ mode: 'unavailable' })).it, explain);
  assert.match(statusText(status({ mode: 'off', enabled: false })).it, /^Spenta/);
  assert.equal(statusText(status({ mode: 'paused_for_subscription' })).it.startsWith("La sincronizzazione riprende con l'abbonamento attivo."), true);
  assert.match(statusText(status({ mode: 'error' })).it, /Riprovo da solo/);
  assert.match(statusText(status({ last_sync: 1_800_000_000 }), (s) => `t${s}`).it, /Ultimo allineamento: t1800000000\./);
  assert.match(statusText(status()).en, /First synchronization/);
});

test('the general switch can be turned on only after the consent', () => {
  assert.equal(canTurnOn(status({ mode: 'off', enabled: false, consented: true })), true);
  assert.equal(canTurnOn(status({ mode: 'off', enabled: false, consented: false })), false);
  assert.equal(canTurnOn(status({ mode: 'no_server', consented: false })), false);
  assert.equal(canTurnOn(null), false);
});

test('the removal text names the mailbox', () => {
  const text = removalText({ item_id: 'x', name: 'Giulia', email: 'giulia@gmail.com' });
  assert.equal(text.it, 'La casella giulia@gmail.com è stata rimossa o esclusa dalla sincronizzazione su un altro computer.');
  assert.match(text.en, /was removed or switched off on another computer/);
});

test('a revocation result says what to do next', () => {
  const base = { account_id: 1, email: 'giulia@gmail.com', provider_id: 'gmail' };
  assert.match(revokeText({ ...base, outcome: 'revoked' }).it, /Ricollega la casella su questo computer/);
  assert.match(revokeText({ ...base, outcome: 'failed' }).it, /revoca non riuscita.*Riprova.*permessi/);
  assert.match(revokeText({ ...base, outcome: 'manual_password' }).it, /Account Google → Password per le app/);
  assert.doesNotMatch(revokeText({ ...base, provider_id: 'fastmail', outcome: 'manual_password' }).it, /Google/);
  assert.equal(GOOGLE_PERMISSIONS_URL, 'https://myaccount.google.com/permissions');
  assert.deepEqual(retryable([{ ...base, outcome: 'failed' }, { ...base, account_id: 2, outcome: 'revoked' }]).map((r) => r.account_id), [1]);
});

test('the revoke checkbox is checked for another device and absent for this one', () => {
  assert.deepEqual(revokeChoice({ current: false }), { shown: true, checked: true });
  assert.deepEqual(revokeChoice({ current: true }), { shown: false, checked: false });
});

test('the interface learns of changed mailboxes from the event', () => {
  assert.deepEqual(parseAppEvent({ kind: 'accounts-changed' }), { kind: 'accounts-changed' });
  assert.equal(mailboxesChanged({ kind: 'accounts-changed' }), true);
  assert.equal(mailboxesChanged({ kind: 'notice' }), false);
});

test('the backend texts of the synchronization are translated', () => {
  assert.equal(backendTextItalian('Mailbox synchronization needs an active Qamille subscription'), "La sincronizzazione delle caselle riprende con l'abbonamento attivo.");
  assert.equal(backendTextItalian('Google did not revoke the access'), "Google non ha revocato l'accesso.");
  assert.equal(backendTextItalian('The mailbox giulia@gmail.com was removed or switched off on another computer'), 'La casella giulia@gmail.com è stata rimossa o esclusa dalla sincronizzazione su un altro computer.');
  assert.equal(backendTextItalian('The server has reached its maximum of synchronized mailboxes').includes('massimo'), true);
});
