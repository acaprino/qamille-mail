// Who "Reply all" writes to (src/lib/reply-all.ts): the sender and the other people the message went to, then
// the people in copy, without the reader's own addresses and without anybody twice.
import assert from 'node:assert/strict';
import test from 'node:test';
import { replyAllRecipients } from '../src/lib/reply-all.ts';

const own = new Set(['me@work.test', 'me@home.test']);

test('the sender comes first, then the other recipients; the reader is taken out', () => {
  const out = replyAllRecipients(
    { to: ['Me <ME@work.test>', 'ann@x.test', 'bob@x.test'], cc: [] },
    'Sam <sam@x.test>',
    own,
  );
  assert.deepEqual(out, { to: 'Sam <sam@x.test>, ann@x.test, bob@x.test', cc: '' });
});

test('every address of the reader is removed, in To and in Cc, whatever the case', () => {
  const out = replyAllRecipients(
    { to: ['me@home.test', 'ann@x.test'], cc: ['Me <Me@Work.test>', 'cy@x.test'] },
    'sam@x.test',
    own,
  );
  assert.deepEqual(out, { to: 'sam@x.test, ann@x.test', cc: 'cy@x.test' });
});

test('nobody appears twice in To, and what is in To is not repeated in Cc', () => {
  const out = replyAllRecipients(
    { to: ['SAM@x.test', 'ann@x.test', 'Ann <ann@x.test>'], cc: ['sam@x.test', 'ann@x.test', 'cy@x.test'] },
    'sam@x.test',
    own,
  );
  assert.deepEqual(out, { to: 'sam@x.test, ann@x.test', cc: 'cy@x.test' });
});

test('Cc is de-duplicated on its own too', () => {
  const out = replyAllRecipients(
    { to: [], cc: ['cy@x.test', 'Cy <CY@x.test>', 'di@x.test', 'cy@x.test'] },
    'sam@x.test',
    own,
  );
  assert.deepEqual(out, { to: 'sam@x.test', cc: 'cy@x.test, di@x.test' });
});

test('when only the reader is left in To, the plain reply target stays', () => {
  const out = replyAllRecipients({ to: ['me@work.test'], cc: [] }, 'me@home.test', own);
  assert.deepEqual(out, { to: 'me@home.test', cc: '' });
});

test('when the reply target is the reader but others are in To, they are kept', () => {
  const out = replyAllRecipients({ to: ['me@work.test', 'ann@x.test'], cc: [] }, 'me@home.test', own);
  assert.deepEqual(out, { to: 'ann@x.test', cc: '' });
});
