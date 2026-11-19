// Names for the people on a message (spec 2026-11-07 §4.4): the name already known for an
// address, the reader first, and a short list with the rest counted.
import assert from 'node:assert/strict';
import test from 'node:test';
import { addressOf, displayName, personIndex, recipientView } from '../src/lib/people.ts';

const index = personIndex([
  { name: 'Ada Lovelace', address: 'Ada@Example.test' },
  { name: 'Grace Hopper', address: 'grace@example.test' },
  { name: '', address: 'nameless@example.test' },
]);

test('the index is keyed by the lowercase address and skips empty names', () => {
  assert.equal(index.get('ada@example.test'), 'Ada Lovelace');
  assert.equal(index.has('nameless@example.test'), false);
  assert.equal(index.size, 2);
});

test('addressOf reads the address out of "Name <address>" and plain forms', () => {
  assert.equal(addressOf('Ada <Ada@Example.test>'), 'ada@example.test');
  assert.equal(addressOf('  grace@example.test '), 'grace@example.test');
  assert.equal(addressOf('<x@y.test>'), 'x@y.test');
});

test('a known address shows its name, an unknown one its address', () => {
  assert.equal(displayName(index, 'ada@example.test'), 'Ada Lovelace');
  assert.equal(displayName(index, 'ADA@example.test'), 'Ada Lovelace', 'the lookup ignores case');
  assert.equal(displayName(index, 'stranger@example.test'), 'stranger@example.test');
  assert.equal(displayName(index, 'nameless@example.test'), 'nameless@example.test');
});

test('a name written in the recipient itself is used when the index has none', () => {
  assert.equal(displayName(index, 'Linus T <linus@example.test>'), 'Linus T');
  assert.equal(displayName(index, 'Someone Else <grace@example.test>'), 'Grace Hopper', 'the index wins');
});

test('the reader comes first and the rest are counted', () => {
  const addresses = Array.from({ length: 40 }, (_, i) => `person${String(i + 1).padStart(2, '0')}@example.test`);
  addresses[4] = 'Alex@Studio.test'; // the fifth recipient is the reader
  addresses[1] = 'grace@example.test';
  const view = recipientView(addresses, index, new Set(['alex@studio.test']), 'you');
  assert.deepEqual(view.shown, ['you', 'person01@example.test', 'Grace Hopper']);
  assert.equal(view.more, 37);
  assert.equal(view.all.length, 40);
  assert.equal(view.all[0], 'you');
  assert.equal(view.all.filter((name) => name === 'you').length, 1, 'the reader appears once');
});

test('several addresses of the reader are one person, and a repeated address is listed once', () => {
  const view = recipientView(
    ['a@mine.test', 'b@theirs.test', 'B@theirs.test', 'c@mine.test'],
    new Map(),
    new Set(['a@mine.test', 'c@mine.test']),
    'you',
  );
  assert.deepEqual(view.all, ['you', 'b@theirs.test']);
  assert.equal(view.more, 0);
});

test('a short list shows everyone and counts nothing', () => {
  const view = recipientView(['grace@example.test', 'x@y.test'], index, new Set(), 'you');
  assert.deepEqual(view.shown, ['Grace Hopper', 'x@y.test']);
  assert.equal(view.more, 0);
});

test('nobody to show is an empty view', () => {
  assert.deepEqual(recipientView([], index, new Set(), 'you'), { shown: [], more: 0, all: [] });
});

test('the number shown can be chosen', () => {
  const view = recipientView(['a@x.test', 'b@x.test', 'c@x.test', 'd@x.test'], new Map(), new Set(), 'you', 2);
  assert.deepEqual(view.shown, ['a@x.test', 'b@x.test']);
  assert.equal(view.more, 2);
});
