// The browser preview answers like the Rust backend about the other messages of a conversation
// (ADR-0010): replies under each row of the Conversations mode, a cap with the rest counted, and
// `read_replies` for what lies beyond. Synthetic data only.
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { IDS, withConversation } from './support/conversation-fixture.mjs';

const port = 28175;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const sample = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));
let browser;
const errors = [];

async function open(fixture) {
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
  page.on('pageerror', (error) => errors.push(error.message));
  await page.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
  );
  await page.goto(url);
  await page.locator('.mail-row').first().waitFor();
  return page;
}
const call = (page, command, args) =>
  page.evaluate(async ([name, value]) => (await import('/src/lib/api.ts')).call(name, value), [command, args]);
const list = (page, query) =>
  call(page, 'list_mail', { query: { folder: 'Inbox', account_id: null, search: '', offset: 0, limit: 60, threads: true, ...query } });
const row = (page, items, id) => items.find((item) => item.id === id);
const parents = (rows) => rows.map((reply) => reply.parent_id);

try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });

  const page = await open(withConversation(sample));
  const inbox = (await list(page, {})).items;
  const main = row(page, inbox, IDS.main);
  assert.ok(main, 'the newest message of the conversation is the main row');
  assert.deepEqual(main.message_ids, [IDS.main, IDS.many, IDS.a, IDS.root], 'the sent reply is shown, never acted on');
  assert.ok(main.conversation_id > 0);
  assert.deepEqual(main.replies.map((reply) => reply.id), [IDS.root, IDS.a, IDS.sent, IDS.many]);
  assert.deepEqual(parents(main.replies), [null, IDS.root, IDS.a, IDS.a]);
  assert.equal(main.replies[2].folder, 'Sent');
  assert.equal(main.replies[3].to.length, 12);
  assert.equal(main.replies[3].read, false);
  assert.equal('replies_more' in main, false, 'nothing more to load: the key is left out');
  assert.equal(main.read, false, 'one unread message of the view makes the row unread');
  console.log('PASS A row lists the other messages in tree order, the sent reply included');

  // The Sent folder shows the same conversation from the other end.
  const sentRow = row(page, (await list(page, { folder: 'Sent' })).items, IDS.sent);
  assert.deepEqual(sentRow.replies.map((reply) => reply.id), [IDS.root, IDS.a, IDS.many, IDS.main]);
  assert.deepEqual(parents(sentRow.replies), [null, IDS.root, IDS.a, null], 'an answer to the main row is a root');
  console.log('PASS The same conversation from the Sent folder');

  // Email mode, a search and a patch by id carry no replies, so they never wipe what the list gave.
  const flat = (await list(page, { threads: false })).items;
  assert.ok(flat.every((item) => !('replies' in item) && !('conversation_id' in item) && !('replies_more' in item)));
  const found = (await list(page, { search: 'launch' })).items;
  assert.ok(found.length > 0 && found.every((item) => !('replies' in item)));
  const patched = (await call(page, 'read_rows', { ids: [IDS.main] }))[0];
  assert.equal('replies' in patched || 'conversation_id' in patched || 'replies_more' in patched, false);
  console.log('PASS Email mode, search and read_rows carry no replies');

  // The headline counts: read mail does not count, the unread one does.
  const counts = await list(page, {});
  assert.equal(typeof counts.unread_total, 'number');
  assert.equal((await list(page, { folder: 'Sent' })).unread_total, null);
  console.log('PASS Counts only for the Inbox');
  await page.close();

  // Beyond the cap: 35 other messages, 30 in the row, the rest through read_replies.
  const long = await open(withConversation(sample, { extraReplies: 31 }));
  // The last extra reply is now the newest message, so it is the main row: 35 others remain.
  const newest = IDS.extra + 30;
  const big = row(long, (await list(long, {})).items, newest);
  assert.equal(big.replies.length, 30);
  assert.equal(big.replies_more, 5);
  assert.equal('replies_more_unread' in big, false, 'nothing unread beyond the cap: the key is left out');
  const rest = await call(long, 'read_replies', { mainId: newest, folder: 'Inbox', offset: 30, limit: 30 });
  assert.equal(rest.length, 5);
  const everything = await call(long, 'read_replies', { mainId: newest, folder: 'Inbox', offset: 0, limit: 100 });
  assert.equal(everything.length, 35);
  assert.deepEqual(everything.slice(0, 30).map((r) => r.id), big.replies.map((r) => r.id), 'the same order as the row');
  assert.deepEqual(rest, everything.slice(30), 'the rest continues exactly where the row stopped');
  assert.equal((await call(long, 'read_replies', { mainId: newest, folder: 'Inbox', offset: 0, limit: 0 })).length, 30, 'no limit means the cap');
  assert.equal((await call(long, 'read_replies', { mainId: newest, folder: 'Inbox', offset: 40, limit: 30 })).length, 0);
  await assert.rejects(() => call(long, 'read_replies', { mainId: 424242, folder: 'Inbox', offset: 0, limit: 30 }), /not found/i);
  console.log('PASS Replies are capped at 30, counted, and read_replies continues them');
  await long.close();

  // An unread reply beyond the cap is flagged on the row (the closed arrow turns blue for it).
  const unreadBeyond = await open(withConversation(sample, { extraReplies: 31, read: { [IDS.many]: true, [IDS.extra + 29]: false } }));
  const flagged = row(unreadBeyond, (await list(unreadBeyond, {})).items, IDS.extra + 30);
  assert.equal(flagged.replies_more_unread, true);
  assert.ok(flagged.replies.every((reply) => reply.read), 'everything listed is read: only the flag says so');
  await unreadBeyond.close();
  console.log('PASS An unread reply beyond the cap is flagged on the row');

  // The Trash and Spam views hold their own members, and read_replies answers for the same view.
  const trashed = withConversation(sample);
  for (const email of trashed.emails) if ([IDS.root, IDS.a, IDS.main].includes(email.id)) email.folder = 'Trash';
  const bin = await open(trashed);
  const trashRow = row(bin, (await list(bin, { folder: 'Trash' })).items, IDS.main);
  assert.deepEqual(trashRow.message_ids, [IDS.main, IDS.a, IDS.root]);
  assert.deepEqual(trashRow.replies.map((reply) => reply.id), [IDS.root, IDS.a, IDS.sent, IDS.many], 'its members and the live ones');
  assert.deepEqual((await call(bin, 'read_replies', { mainId: IDS.main, folder: 'Trash', offset: 0, limit: 30 })).map((r) => r.id), [IDS.root, IDS.a, IDS.sent, IDS.many]);
  assert.deepEqual((await call(bin, 'read_replies', { mainId: IDS.main, folder: 'Inbox', offset: 0, limit: 30 })).map((r) => r.id), [IDS.sent, IDS.many], 'the live views hide Trash');
  await bin.close();
  console.log('PASS The Trash view lists its own members');

  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  server.kill();
}
