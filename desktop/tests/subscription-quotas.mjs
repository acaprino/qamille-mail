// The consumption of the month, the pauses of a used-up quota, the warning before deleting an
// account that pays and the Terms of a new account (wave 9): the pure functions of
// src/lib/subscription-usage.ts.
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  acceptedTerms,
  biggerPlan,
  deleteWarning,
  exhausted,
  exhaustedKeys,
  legalAddress,
  needsTermsBox,
  percent,
  quotaName,
  quotaPauseText,
  quotaResource,
  renewalText,
  termsBlock,
  usageText,
} from '../src/lib/subscription-usage.ts';

const RENEWS = '2026-12-01T00:00:00.000Z';
const usage = (over = {}) => ({
  period_start: '2026-11-01',
  renews_at: RENEWS,
  audio_minutes: { used: 12, limit: 600 },
  embedded_tokens: { used: 1000, limit: 2000000 },
  llm_tokens: { used: 5, limit: 100 },
  ...over,
});

test('the usage says what was used and what the plan gives, in both languages', () => {
  assert.deepEqual(usageText('audio_minutes', { used: 12, limit: 600 }), { it: '12 di 600 minuti', en: '12 of 600 minutes' });
  const tokens = usageText('embedded_tokens', { used: 1500, limit: 2000000 });
  assert.equal(tokens.en, '1,500 of 2,000,000 tokens');
  assert.equal(tokens.it, '1500 di 2.000.000 token');
  assert.equal(quotaName('llm_tokens').it, 'Assistente');
  assert.equal(quotaName('embedded_tokens').en, 'Semantic search');
});

test('the bar never leaves 0 to 100 and a limit of zero reads as full', () => {
  assert.equal(percent({ used: 0, limit: 100 }), 0);
  assert.equal(percent({ used: 50, limit: 100 }), 50);
  assert.equal(percent({ used: 999, limit: 1000 }), 99, 'never rounded up to «full» before it is');
  assert.equal(percent({ used: 500, limit: 100 }), 100);
  assert.equal(percent({ used: 0, limit: 0 }), 100);
  assert.equal(exhausted({ used: 100, limit: 100 }), true);
  assert.equal(exhausted({ used: 99, limit: 100 }), false);
  assert.deepEqual(exhaustedKeys(usage({ llm_tokens: { used: 100, limit: 100 } })), ['llm_tokens']);
  assert.deepEqual(exhaustedKeys(null), []);
  assert.deepEqual(exhaustedKeys(usage()), []);
});

test('the renewal day is the first of the month in UTC, with the Italian ordinal', () => {
  assert.deepEqual(renewalText(RENEWS), { it: 'Si azzera il 1° dicembre.', en: 'Resets on 1 December.' });
  assert.equal(renewalText('2027-01-01T00:00:00.000Z').it, 'Si azzera il 1° gennaio.');
  assert.equal(renewalText('2026-12-15T00:00:00.000Z').it, 'Si azzera il 15 dicembre.');
  assert.equal(renewalText('not a date'), null);
  assert.equal(renewalText(null), null);
});

test('a used-up quota says which service stops and when it comes back', () => {
  assert.match(quotaPauseText('llm', RENEWS).it, /^L'assistente è fermo.* Tornano il 1° dicembre\.$/);
  assert.match(quotaPauseText('transcription', RENEWS).en, /^Transcription is paused.* They come back on 1 December\.$/);
  assert.match(quotaPauseText('compute', RENEWS).it, /^Indicizzazione e ricerca per significato sono ferme/);
  assert.match(quotaPauseText('compute').it, /Tornano all'inizio del mese prossimo\.$/, 'without a date it still says when');
  assert.equal(quotaResource('embedded_tokens'), 'compute');
  assert.equal(quotaResource('audio_minutes'), 'transcription');
  assert.equal(quotaResource('llm_tokens'), 'llm');
});

test('a bigger plan is offered only when the server declares one that gives more of the full meter', () => {
  const plan = (id, price, quotas) => ({ id, name: id, interval: 'month', price_cents: price, currency: 'EUR', quotas });
  const small = plan('small', 900, { audio_minutes: 600, embedded_tokens: 2000000, llm_tokens: 100 });
  const medium = plan('medium', 1900, { audio_minutes: 1200, embedded_tokens: 2000000, llm_tokens: 500 });
  const large = plan('large', 4900, { audio_minutes: 3000, embedded_tokens: 8000000, llm_tokens: 2000 });
  assert.equal(biggerPlan([small, medium, large], 'llm_tokens', 100).id, 'medium', 'the cheapest that gives more');
  assert.equal(biggerPlan([small, medium, large], 'embedded_tokens', 2000000).id, 'large', 'medium gives the same search tokens');
  assert.equal(biggerPlan([small, medium, large], 'llm_tokens', 2000), null, 'already the largest');
  assert.equal(biggerPlan([plan('a', 100, null), plan('b', 200, undefined)], 'llm_tokens', 1), null, 'a server without quotas on its plans');
  assert.equal(biggerPlan([], 'llm_tokens', 1), null);
});

test('deleting an account warns only when a subscription pays at a processor', () => {
  const sub = (over) => ({ status: 'active', entitled: true, provider: 'stripe', plan_id: 'pro', manageable: true, ...over });
  for (const status of ['active', 'past_due']) {
    const warning = deleteWarning(sub({ status }));
    assert.match(warning.it, /lo disdice subito presso il processore/);
    assert.match(warning.en, /cancel it at once with the payment processor/);
    assert.match(warning.en, /not refunded automatically/);
  }
  assert.equal(deleteWarning(sub({ provider: 'manual' })), null, 'granted by hand: nothing at a processor');
  assert.deepEqual(deleteWarning(sub({ status: 'trialing', provider: 'trial' })), { it: "La prova finisce con l'account.", en: 'The trial ends with the account.' });
  assert.equal(deleteWarning(sub({ provider: 'trial' })).it, "La prova finisce con l'account.");
  for (const status of ['none', 'canceled', 'expired']) assert.equal(deleteWarning(sub({ status })), null, status);
  assert.equal(deleteWarning(null), null);
  assert.equal(deleteWarning(undefined), null);
});

test('the Terms box exists only for a server that declares its documents, and sends the version it declared', () => {
  const legal = { terms_url: 'https://q.example/terms', privacy_url: 'https://q.example/privacy', terms_version: '2026-12-01' };
  assert.equal(needsTermsBox(legal), true);
  for (const none of [null, undefined, {}, { terms_url: null, privacy_url: null, terms_version: null }, { terms_url: 'http://q.example/terms' }]) {
    assert.equal(needsTermsBox(none), false, JSON.stringify(none));
  }
  assert.equal(needsTermsBox({ terms_version: '2026-12-01' }), true, 'a version alone still has to be accepted');
  assert.equal(termsBlock(legal, false), true, 'without the tick the sign-up waits');
  assert.equal(termsBlock(legal, true), false);
  assert.equal(termsBlock(null, false), false, 'a server without documents asks nothing');
  assert.equal(acceptedTerms(legal, true), '2026-12-01');
  assert.equal(acceptedTerms(legal, false), null, 'never sent without the tick');
  assert.equal(acceptedTerms({ terms_url: legal.terms_url }, true), null, 'no version declared, nothing to send');
  assert.equal(acceptedTerms(null, true), null);
});

test('the addresses of the documents open only as https without credentials (INV-26)', () => {
  assert.equal(legalAddress('https://q.example/terms'), 'https://q.example/terms');
  for (const bad of ['http://q.example/terms', 'javascript:alert(1)', 'file:///c:/x', 'https://user:pw@q.example/', '//q.example/terms', 'https://q.example/a b', 'https://q.example/\n', '', null, undefined]) {
    assert.equal(legalAddress(bad), null, String(bad));
  }
});
