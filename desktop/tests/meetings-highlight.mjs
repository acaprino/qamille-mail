// Task 12: how a search result shows its fragment (src/lib/meetings-highlight.ts). Pure text in,
// pieces out: the page never builds HTML from what the user typed or what a meeting says.
import assert from 'node:assert/strict';
import test from 'node:test';
import { fragmentAround, markedParts, searchTerms } from '../src/lib/meetings-highlight.ts';

const flagged = (parts) => parts.filter((part) => part.hit).map((part) => part.text);
const rejoined = (parts) => parts.map((part) => part.text).join('');

test('the terms are the words of the query, as the backend reads them', () => {
  assert.deepEqual(searchTerms('  budget   marketing '), ['budget', 'marketing']);
  assert.deepEqual(searchTerms('   '), []);
  assert.deepEqual(searchTerms(''), []);
});

test('a match is found whatever its case and keeps the case of the text', () => {
  const parts = markedParts('Il Budget del budget BUDGET', 'budget');
  assert.deepEqual(flagged(parts), ['Budget', 'budget', 'BUDGET']);
  assert.equal(rejoined(parts), 'Il Budget del budget BUDGET');
});

test('every word of the query is marked, the longest first', () => {
  assert.deepEqual(flagged(markedParts('il prototipo di Maria', 'pro prototipo')), ['prototipo']);
  assert.deepEqual(flagged(markedParts('il prototipo di Maria', 'maria prototipo')), ['prototipo', 'Maria']);
});

test('characters that mean something in a regular expression are only text', () => {
  assert.deepEqual(flagged(markedParts('costa (circa) 3.5 euro', '(circa)')), ['(circa)']);
  assert.deepEqual(flagged(markedParts('costa a.b e axb', 'a.b')), ['a.b']);
  assert.doesNotThrow(() => markedParts('testo', '[unclosed'));
  assert.doesNotThrow(() => markedParts('testo', '\\'));
});

test('without a query, or without a match, the text is one plain piece', () => {
  assert.deepEqual(markedParts('niente da vedere', ''), [{ text: 'niente da vedere', hit: false }]);
  assert.deepEqual(markedParts('niente da vedere', 'altro'), [{ text: 'niente da vedere', hit: false }]);
  assert.deepEqual(markedParts('', 'x'), []);
});

test('markup in the text stays text', () => {
  const parts = markedParts('<b>budget</b> & <script>x</script>', 'budget');
  assert.deepEqual(flagged(parts), ['budget']);
  assert.equal(rejoined(parts), '<b>budget</b> & <script>x</script>');
});

test('a short text is shown whole, with its spaces tidied', () => {
  assert.equal(fragmentAround('Una frase   breve\ncon il budget.', 'budget'), 'Una frase breve con il budget.');
});

test('a long text is cut around the first match, with an ellipsis where it was cut', () => {
  const text = `${'prima '.repeat(60)}il budget marketing passa al dieci per cento ${'dopo '.repeat(60)}`;
  const cut = fragmentAround(text, 'budget', 120);
  assert.ok(cut.startsWith('…'), cut);
  assert.ok(cut.endsWith('…'), cut);
  assert.ok(cut.includes('budget'), cut);
  assert.ok(cut.length <= 120 + 2, `${cut.length}`);
});

test('a match near the start has no ellipsis in front', () => {
  const text = `Il budget è il primo punto. ${'resto '.repeat(80)}`;
  const cut = fragmentAround(text, 'budget', 100);
  assert.ok(!cut.startsWith('…'), cut);
  assert.ok(cut.endsWith('…'), cut);
  assert.ok(cut.startsWith('Il budget'), cut);
});

test('without a match the text starts from the beginning', () => {
  const cut = fragmentAround(`${'parole '.repeat(60)}`, 'assente', 80);
  assert.ok(cut.startsWith('parole'), cut);
  assert.ok(cut.endsWith('…'), cut);
});
