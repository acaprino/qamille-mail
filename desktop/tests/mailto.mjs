// mailto: links (RFC 6068) and the way they reach the composer (src/lib/mailto.ts, docs/aspetto-nativo.md
// rule 32, audit Q10). A link is untrusted input: it fills a new message, never sends one, never attaches
// a file and never adds a header beyond To, Cc, Bcc, Subject and the text.
// Run: node --test tests/mailto.mjs (from desktop/). Node strips the type annotations of this
// dependency-free module.
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  MAILTO_LIMIT,
  installMailtoRouting,
  isMailto,
  mailtoDraft,
  onComposeRequest,
  parseMailto,
  requestCompose,
  validAddress,
} from '../src/lib/mailto.ts';

const empty = { to: [], cc: [], bcc: [], subject: '', body: '' };

test('recipients from the path and from to, cc and bcc', () => {
  assert.deepEqual(parseMailto('mailto:a@b.it'), { ...empty, to: ['a@b.it'] });
  assert.deepEqual(parseMailto('MAILTO:a@b.it,c@d.it'), { ...empty, to: ['a@b.it', 'c@d.it'] });
  // Percent-encoded in the path and in the headers; header names in any case, themselves encoded too.
  assert.deepEqual(parseMailto('mailto:a%40b.it?To=e@f.it,%20g@h.it&cc=i@l.it&%62cc=m@n.it&CC=o@p.it'), {
    ...empty,
    to: ['a@b.it', 'e@f.it', 'g@h.it'],
    cc: ['i@l.it', 'o@p.it'],
    bcc: ['m@n.it'],
  });
  // No path: the recipients come from the headers only.
  assert.deepEqual(parseMailto('mailto:?to=a@b.it'), { ...empty, to: ['a@b.it'] });
  // An address twice in one field is there once (case does not matter), as the composer keeps it.
  assert.deepEqual(parseMailto('mailto:a@b.it?to=A@B.IT'), { ...empty, to: ['a@b.it'] });
  // Each address passes the composer's own check; one that fails is left out, the others stay.
  assert.deepEqual(parseMailto('mailto:not-an-address,a@b.it?cc=x%20y@z.it,<c@d.it>,e@f.it'), {
    ...empty,
    to: ['a@b.it'],
    cc: ['e@f.it'],
  });
  assert.equal(validAddress('a@b.it'), true);
  for (const refused of ['', 'a', 'a@', '@b.it', 'a b@c.it', 'a@b@c.it']) assert.equal(validAddress(refused), false, refused);
  // A bare "mailto:" is a new message with nothing in it.
  assert.deepEqual(parseMailto('mailto:'), empty);
});

test('subject and body are decoded, plus stays a plus', () => {
  const fields = parseMailto('mailto:a@b.it?subject=Caff%C3%A8%20alle%203+1&body=Riga%201%0D%0ARiga%202%0A%0Ac%2Bd+e');
  assert.equal(fields.subject, 'Caffè alle 3+1');
  // The text keeps its lines (CR LF become LF) and its plus signs: mailto: is not form encoding.
  assert.equal(fields.body, 'Riga 1\nRiga 2\n\nc+d+e');
  // The first subject and the first body count; a later one does not replace them.
  assert.deepEqual(parseMailto('mailto:?subject=one&subject=two&body=first&body=second'), { ...empty, subject: 'one', body: 'first' });
  // What a URL parser would call a fragment is not part of a mailto: address (RFC 6068: "#" is encoded).
  assert.deepEqual(parseMailto('mailto:a@b.it?subject=x#y'), { ...empty, to: ['a@b.it'], subject: 'x' });
  assert.equal(parseMailto('mailto:a@b.it?subject=%23x').subject, '#x');
  // A percent sign that starts no escape makes the address unreadable: nothing is guessed.
  assert.equal(parseMailto('mailto:a@b.it?subject=100%'), null);
  assert.equal(parseMailto('mailto:a@b.it?subject=%E0%A4%A'), null);
  // The draft the composer gets: the text as plain text, which it shows as any plain-text draft.
  assert.deepEqual(mailtoDraft({ to: ['a@b.it', 'c@d.it'], cc: ['e@f.it'], bcc: [], subject: 's', body: 'b\nc' }), {
    to: 'a@b.it, c@d.it',
    cc: 'e@f.it',
    bcc: '',
    subject: 's',
    body: 'b\nc',
    format: 'Plain',
  });
});

test('a line break cannot add a header or a recipient', () => {
  const fields = parseMailto('mailto:a@b.it?subject=x%0D%0ABcc:c@d.it');
  assert.deepEqual(fields.to, ['a@b.it'], 'one recipient');
  assert.deepEqual(fields.cc, []);
  assert.deepEqual(fields.bcc, [], 'no hidden recipient');
  assert.equal(fields.subject, 'x Bcc:c@d.it', 'the subject stays on one line');
  // In a recipient a line break leaves an address that is not one: it is left out.
  assert.deepEqual(parseMailto('mailto:a@b.it%0D%0ABcc:c@d.it?to=e@f.it%0A,g@h.it').to, ['e@f.it', 'g@h.it']);
  assert.deepEqual(parseMailto('mailto:?cc=a@b.it%0D%0AX-Evil:1').cc, []);
  // Other control characters never reach a header either; the text keeps its tabs and lines only.
  assert.equal(parseMailto('mailto:?subject=a%00b%09c%1Bd').subject, 'ab\tcd');
  assert.deepEqual(parseMailto('mailto:a%00@b.it').to, []);
  assert.equal(parseMailto('mailto:?body=a%00b%0Dc%09d').body, 'ab\nc\td');
});

test('unknown headers are ignored and a non-mailto address is null', () => {
  // No attachment, sender, reply address or header of the link's own: only to, cc, bcc, subject, body.
  assert.deepEqual(
    parseMailto('mailto:a@b.it?attach=C:%5Cx.exe&attachment=x&from=boss@x.it&reply-to=r@x.it&in-reply-to=%3Cid%3E&x-mailer=y&subject=s'),
    { ...empty, to: ['a@b.it'], subject: 's' },
  );
  // A header without a value, or an empty pair, changes nothing.
  assert.deepEqual(parseMailto('mailto:a@b.it?cc&&subject='), { ...empty, to: ['a@b.it'] });
  for (const other of ['https://example.com/', 'mailto', 'mail:a@b.it', ' mailto:a@b.it', 'javascript:mailto:a@b.it', '']) {
    assert.equal(parseMailto(other), null, JSON.stringify(other));
  }
  assert.equal(isMailto('MailTo:a@b.it'), true);
  assert.equal(isMailto('https://a.it/mailto:a@b.it'), false);
});

test('an address over 8 KB is refused', () => {
  assert.equal(MAILTO_LIMIT, 8 * 1024);
  const head = 'mailto:a@b.it?body=';
  const exactly = head + 'x'.repeat(MAILTO_LIMIT - head.length);
  assert.equal(parseMailto(exactly).body.length, MAILTO_LIMIT - head.length);
  assert.equal(parseMailto(exactly + 'x'), null);
  // The limit counts bytes (UTF-8), as the address travels.
  const wide = head + 'è'.repeat((MAILTO_LIMIT - head.length) / 2);
  assert.notEqual(parseMailto(wide), null);
  assert.equal(parseMailto(wide + 'è'), null);
});

test('a request to compose waits for the composer, then goes to it', () => {
  const fields = { ...empty, to: ['a@b.it'] };
  // A message link clicked before the app listens: kept, then handed over once.
  requestCompose(fields);
  const got = [];
  const stop = onComposeRequest((request) => got.push(request));
  assert.deepEqual(got, [fields]);
  requestCompose({ ...empty, subject: 'two' });
  assert.equal(got.length, 2);
  stop();
  const later = [];
  const again = onComposeRequest((request) => later.push(request));
  assert.deepEqual(later, [], 'what the first listener got is not given again');
  again();
});

/** A fake Tauri: `listen` resolves when `attach()` is called, `call` answers take_launch_mailto. */
function fakeNative(launch) {
  const log = [];
  let handler = null;
  let attach;
  const attached = new Promise((resolve) => (attach = resolve));
  return {
    log,
    attach: () => attach(),
    emit: (payload) => handler?.({ payload }),
    native: {
      listen: async (name, fn) => {
        log.push(`listen ${name}`);
        await attached;
        handler = fn;
        return () => {
          log.push(`unlisten ${name}`);
          handler = null;
        };
      },
      call: async (command) => {
        log.push(command);
        return launch;
      },
    },
  };
}
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test('a link from another program waits until the page listens, and opens the composer once', async () => {
  const opened = [];
  const refused = [];
  const fake = fakeNative('mailto:a@b.it?subject=launch');
  const stop = installMailtoRouting({
    compose: (fields) => opened.push(fields),
    refused: () => refused.push('refused'),
    failed: (error) => assert.fail(String(error)),
    native: fake.native,
  });
  await settle();
  // The link of the start is asked for only once the listener of later links is in.
  assert.deepEqual(fake.log, ['listen compose-mailto']);
  fake.attach();
  await settle();
  assert.deepEqual(fake.log, ['listen compose-mailto', 'take_launch_mailto']);
  assert.deepEqual(opened, [{ ...empty, to: ['a@b.it'], subject: 'launch' }]);
  // A second start's link arrives as an event; one that is not a mailto: address opens nothing.
  fake.emit('mailto:c@d.it');
  fake.emit('https://example.com/');
  assert.deepEqual(opened.slice(1), [{ ...empty, to: ['c@d.it'] }]);
  assert.deepEqual(refused, ['refused']);
  // The message links of the page reach the same composer.
  requestCompose({ ...empty, subject: 'message' });
  assert.equal(opened.at(-1).subject, 'message');
  stop();
  assert.ok(fake.log.includes('unlisten compose-mailto'));
  requestCompose({ ...empty, subject: 'after' });
  assert.notEqual(opened.at(-1).subject, 'after');
  // That request is kept for the next composer: taken here, so it does not reach the next test's.
  const kept = [];
  onComposeRequest((request) => kept.push(request.subject))();
  assert.deepEqual(kept, ['after']);
  // Nothing waiting at the start: no composer.
  const quiet = fakeNative(null);
  const none = [];
  const off = installMailtoRouting({ compose: (fields) => none.push(fields), refused: () => none.push('refused'), failed: () => {}, native: quiet.native });
  quiet.attach();
  await settle();
  assert.deepEqual(none, []);
  off();
});

test('a routing removed before the listener is in never takes the link of the start', async () => {
  const fake = fakeNative('mailto:a@b.it');
  const opened = [];
  const stop = installMailtoRouting({ compose: (fields) => opened.push(fields), refused: () => {}, failed: () => {}, native: fake.native });
  stop();
  fake.attach();
  await settle();
  // The page that went (a reload) leaves the link to the next one.
  assert.deepEqual(fake.log, ['listen compose-mailto', 'unlisten compose-mailto']);
  assert.deepEqual(opened, []);
});
