import assert from 'node:assert/strict';
import test from 'node:test';
import {
  applyEvent,
  applyRows,
  carriesFlags,
  coalescing,
  parseAppEvent,
  readRowsMax,
  rowMatches,
  sameFilter,
} from '../src/lib/events.ts';

const deepFreeze = (value) => {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    Object.values(value).forEach(deepFreeze);
  }
  return value;
};
// Rows are listed newest first, like the backend does: by date, then by id, both descending.
const row = (id, overrides = {}) => ({
  id,
  message_ids: [id],
  account_id: 1,
  folder: 'Inbox',
  from_name: 'Ada',
  from_addr: 'ada@example.com',
  subject: `Message ${id}`,
  snippet: '',
  date: 1000 - id,
  read: false,
  pinned: false,
  done: false,
  category: 'Personal',
  section: 'New',
  labels: [],
  attachment_count: 0,
  due: null,
  // What read_rows always carries (see MailRow): null / false when unset.
  snoozed_until: null,
  set_aside: false,
  reminder_at: null,
  ...overrides,
});
const pageOf = (items, overrides = {}) => ({
  items,
  total: items.length,
  next_offset: null,
  ...overrides,
});
const undo = (id, overrides = {}) => ({
  id,
  operation: { kind: 'mail', email_id: id, action: 'archive', value: null },
  subject: `Message ${id}`,
  created_at_ms: 0,
  expires_at_ms: 5000,
  status: 'pending',
  error: null,
  ...overrides,
});
const workspaceOf = (actions = []) => ({ undo_actions: actions, accounts: [], counts: { Inbox: 3 } });
const inbox = (overrides = {}) => ({
  folder: 'Inbox',
  account_id: null,
  search: '',
  threads: false,
  ...overrides,
});
const changed = (ids, account_id = 1) => ({ kind: 'email-changed', account_id, ids });
const ids = (page) => page.items.map((item) => item.id);

test('UndoRemoved takes the action off the workspace without any call', () => {
  const workspace = deepFreeze(workspaceOf([undo(1), undo(2)]));
  const page = deepFreeze(pageOf([row(1)]));
  const result = applyEvent(page, workspace, { kind: 'undo-removed', id: 1 }, inbox());
  assert.deepEqual(
    result.workspace.undo_actions.map((action) => action.id),
    [2],
  );
  assert.equal(result.page, page, 'The page is the same object');
  assert.deepEqual(result.needsRows, []);
  assert.equal(result.reload, false);
  assert.equal(result.snapshot, false);
});

test('UndoRemoved for an action the workspace does not hold changes nothing', () => {
  const workspace = deepFreeze(workspaceOf([undo(1)]));
  const result = applyEvent(pageOf([]), workspace, { kind: 'undo-removed', id: 9 }, inbox());
  assert.equal(result.workspace, workspace);
});

test('UndoChanged replaces the action in place, or adds it', () => {
  const workspace = deepFreeze(workspaceOf([undo(1), undo(2)]));
  const failed = undo(1, { status: 'failed', error: 'offline' });
  const replaced = applyEvent(pageOf([]), workspace, { kind: 'undo-changed', action: failed }, inbox());
  assert.deepEqual(
    replaced.workspace.undo_actions.map((action) => [action.id, action.status]),
    [
      [1, 'failed'],
      [2, 'pending'],
    ],
  );
  const added = applyEvent(pageOf([]), workspace, { kind: 'undo-changed', action: undo(3) }, inbox());
  assert.deepEqual(
    added.workspace.undo_actions.map((action) => action.id),
    [1, 2, 3],
  );
  assert.deepEqual(added.needsRows, []);
});

test('EmailChanged for an id that is not on the page asks for that row while a filter is active', () => {
  const page = deepFreeze(pageOf([row(1), row(2)]));
  const result = applyEvent(page, deepFreeze(workspaceOf()), changed([7]), inbox());
  assert.deepEqual(result.needsRows, [7]);
  assert.equal(result.page, page, 'Nothing is guessed before the row arrives');
  assert.equal(result.reload, false);
});

test('EmailChanged asks once for every id, on the page or not, without repeats', () => {
  const page = deepFreeze(pageOf([row(1), row(2)]));
  const result = applyEvent(page, workspaceOf(), changed([2, 7, 2, 9]), inbox());
  assert.deepEqual(result.needsRows, [2, 7, 9]);
});

test('EmailChanged of another account leaves a single-account list alone', () => {
  const page = deepFreeze(pageOf([row(1)]));
  const result = applyEvent(page, workspaceOf(), changed([1, 7], 2), inbox({ account_id: 1 }));
  assert.deepEqual(result.needsRows, []);
  assert.equal(result.reload, false);
  assert.equal(result.page, page);
  const same = applyEvent(page, workspaceOf(), changed([1], 1), inbox({ account_id: 1 }));
  assert.deepEqual(same.needsRows, [1]);
});

test('a conversation list or a search is reloaded, because its rows cannot be patched one by one', () => {
  const page = pageOf([row(1)]);
  const threads = applyEvent(page, workspaceOf(), changed([1]), inbox({ threads: true }));
  assert.equal(threads.reload, true);
  assert.deepEqual(threads.needsRows, []);
  const search = applyEvent(page, workspaceOf(), changed([1]), inbox({ search: 'budget' }));
  assert.equal(search.reload, true);
  assert.deepEqual(search.needsRows, []);
});

test('an event that touches more rows than one read can answer reloads the list', () => {
  const many = Array.from({ length: readRowsMax + 1 }, (_, index) => index + 1);
  const bulk = applyEvent(pageOf([]), workspaceOf(), changed(many), inbox());
  assert.equal(bulk.reload, true);
  assert.deepEqual(bulk.needsRows, []);
  const limit = applyEvent(pageOf([]), workspaceOf(), changed(many.slice(0, readRowsMax)), inbox());
  assert.equal(limit.reload, false);
  assert.equal(limit.needsRows.length, readRowsMax);
});

test('CountsChanged and DraftsChanged ask for one workspace snapshot, not for a list reload', () => {
  for (const event of [
    { kind: 'counts-changed', account_id: 1 },
    { kind: 'drafts-changed', ids: [4] },
  ]) {
    const result = applyEvent(pageOf([row(1)]), workspaceOf(), event, inbox());
    assert.equal(result.snapshot, true, event.kind);
    assert.equal(result.reload, false, event.kind);
    assert.deepEqual(result.needsRows, [], event.kind);
  }
});

test('progress events and notices change neither the page nor the workspace', () => {
  const page = deepFreeze(pageOf([row(1)]));
  const workspace = deepFreeze(workspaceOf([undo(1)]));
  for (const event of [
    { kind: 'sync-progress', account_id: 1, phase: 'syncing', added: 2 },
    { kind: 'index-progress', account_id: 1, percent: 40 },
    { kind: 'notice', text: 'Moved' },
  ]) {
    const result = applyEvent(page, workspace, event, inbox());
    assert.equal(result.page, page, event.kind);
    assert.equal(result.workspace, workspace, event.kind);
    assert.deepEqual(result.needsRows, [], event.kind);
    assert.equal(result.reload || result.snapshot, false, event.kind);
  }
});

test('applyRows removes a row whose folder changed so it no longer matches the filter', () => {
  const page = deepFreeze(pageOf([row(1), row(2), row(3)], { next_offset: 3, total: 40 }));
  const moved = row(2, { folder: 'Archive' });
  const result = applyRows(page, [2], [moved], inbox(), 0);
  assert.deepEqual(ids(result), [1, 3]);
  assert.equal(result.total, 39, 'The total follows the row that left');
  assert.equal(result.next_offset, 2, '"Load more" still starts right after the last loaded row');
  assert.equal(ids(page).length, 3, 'The input page is not touched');
});

test('applyRows removes a row the backend no longer has', () => {
  const page = deepFreeze(pageOf([row(1), row(2)]));
  const result = applyRows(page, [2], [], inbox(), 0);
  assert.deepEqual(ids(result), [1]);
  assert.equal(result.total, 1);
});

test('applyRows ignores a row that is not on the page and does not match', () => {
  const page = deepFreeze(pageOf([row(1)]));
  const result = applyRows(page, [5], [row(5, { folder: 'Trash' })], inbox(), 0);
  assert.equal(result, page);
});

test('applyRows replaces a row that still matches, keeping its place', () => {
  const page = deepFreeze(pageOf([row(1), row(2), row(3)], { next_offset: 3 }));
  const result = applyRows(page, [2], [row(2, { read: true, subject: 'Edited' })], inbox(), 0);
  assert.deepEqual(ids(result), [1, 2, 3]);
  assert.equal(result.items[1].read, true);
  assert.equal(result.items[1].subject, 'Edited');
  assert.equal(result.total, 3);
  assert.equal(result.next_offset, 3);
});

test('applyRows inserts a row that joins the list at its place in the order', () => {
  const page = deepFreeze(pageOf([row(1), row(3)], { next_offset: 2, total: 9 }));
  const result = applyRows(page, [2], [row(2)], inbox(), 0);
  assert.deepEqual(ids(result), [1, 2, 3]);
  assert.equal(result.total, 10);
  assert.equal(result.next_offset, 3);
});

test('applyRows leaves a row that would sort after the loaded rows for "Load more"', () => {
  const page = deepFreeze(pageOf([row(1), row(2)], { next_offset: 2, total: 9 }));
  const older = row(8, { date: 5 });
  const result = applyRows(page, [8], [older], inbox(), 0);
  assert.equal(result, page);
  const complete = applyRows(pageOf([row(1), row(2)]), [8], [older], inbox(), 0);
  assert.deepEqual(ids(complete), [1, 2, 8], 'Once every row is loaded it goes last');
  assert.equal(complete.total, 3);
});

test('applyRows moves a row whose date changed, and drops it from view when it falls past the loaded rows', () => {
  const page = deepFreeze(pageOf([row(1), row(2), row(3)], { next_offset: 3, total: 9 }));
  const newest = applyRows(page, [3], [row(3, { date: 5000 })], inbox(), 0);
  assert.deepEqual(ids(newest), [3, 1, 2]);
  assert.equal(newest.next_offset, 3);
  const oldest = applyRows(page, [1], [row(1, { date: 1 })], inbox(), 0);
  assert.deepEqual(ids(oldest), [2, 3]);
  assert.equal(oldest.total, 9, 'It still belongs to the folder');
  assert.equal(oldest.next_offset, 2, 'and "Load more" will bring it back');
});

test('rowMatches follows the same rules as the backend list', () => {
  const now = 100;
  const at = (folder, overrides) => rowMatches(row(1, overrides), inbox({ folder }), now);
  assert.equal(at('Inbox', {}), true);
  assert.equal(at('Inbox', { folder: 'Archive' }), false);
  assert.equal(at('Inbox', { done: true }), false);
  assert.equal(at('Inbox', { snoozed_until: 200 }), false, 'Snoozed into the future');
  assert.equal(at('Inbox', { snoozed_until: 50 }), true, 'The snooze is over');
  assert.equal(at('Archive', { folder: 'archive' }), true, 'Folder names ignore case');
  assert.equal(at('Tasks', { pinned: true }), true);
  assert.equal(at('Tasks', { due: 5 }), true);
  assert.equal(at('Tasks', { reminder_at: 5 }), true);
  assert.equal(at('Tasks', { set_aside: true }), true);
  assert.equal(at('Tasks', {}), false);
  assert.equal(at('Tasks', { pinned: true, done: true }), false);
  assert.equal(at('Tasks', { pinned: true, folder: 'Trash' }), false);
  assert.equal(at('Snoozed', { snoozed_until: 200 }), true);
  assert.equal(at('Snoozed', {}), false);
  assert.equal(at('Snoozed', { snoozed_until: 200, folder: 'Spam' }), false);
  assert.equal(at('label:work', { labels: ['work'] }), true);
  assert.equal(at('label:work', { labels: ['home'] }), false);
  assert.equal(at('label:work', { labels: ['work'], folder: 'Trash' }), false);
  assert.equal(rowMatches(row(1, { account_id: 2 }), inbox({ account_id: 1 }), now), false);
  assert.equal(rowMatches(row(1, { account_id: 1 }), inbox({ account_id: 1 }), now), true);
});

test('a snoozed message leaves the Inbox when its row carries snoozed_until', () => {
  const now = 100;
  const page = deepFreeze(pageOf([row(1), row(2), row(3)], { next_offset: 3, total: 3 }));
  const snoozed = applyRows(page, [2], [row(2, { snoozed_until: 500 })], inbox(), now);
  assert.deepEqual(ids(snoozed), [1, 3]);
  // The same row without the key is read as "not snoozed": it stays, which is why the caller
  // refuses such an answer (carriesFlags) instead of patching with it.
  const flagless = row(2);
  delete flagless.snoozed_until;
  delete flagless.set_aside;
  delete flagless.reminder_at;
  assert.deepEqual(ids(applyRows(page, [2], [{ ...flagless, subject: 'x' }], inbox(), now)), [1, 2, 3]);
  assert.equal(carriesFlags([flagless]), false);
});

test('carriesFlags wants snoozed_until, set_aside and reminder_at on every row, null or false counts', () => {
  assert.equal(carriesFlags([]), true);
  assert.equal(carriesFlags([row(1), row(2, { snoozed_until: 5, set_aside: true, reminder_at: 9 })]), true);
  for (const key of ['snoozed_until', 'set_aside', 'reminder_at']) {
    const broken = row(1);
    delete broken[key];
    assert.equal(carriesFlags([row(2), broken]), false, `${key} missing`);
  }
  assert.equal(carriesFlags([null]), false);
  assert.equal(carriesFlags('rows'), false);
  assert.equal(carriesFlags(undefined), false);
});

test('sameFilter tells the view, the account, the search and the display mode apart', () => {
  assert.equal(sameFilter(inbox(), inbox()), true);
  assert.equal(sameFilter(inbox(), inbox({ folder: 'Archive' })), false);
  assert.equal(sameFilter(inbox(), inbox({ account_id: 2 })), false);
  assert.equal(sameFilter(inbox(), inbox({ search: 'a' })), false);
  assert.equal(sameFilter(inbox(), inbox({ threads: true })), false);
});

test('parseAppEvent accepts what the backend serializes and nothing else', () => {
  const valid = [
    { kind: 'email-changed', account_id: 2, ids: [71] },
    { kind: 'counts-changed', account_id: 2 },
    { kind: 'undo-changed', action: undo(4) },
    { kind: 'undo-removed', id: 4 },
    { kind: 'drafts-changed', ids: [8] },
    { kind: 'sync-progress', account_id: 1, phase: 'complete', added: 3 },
    { kind: 'index-progress', account_id: 1, percent: 40 },
    { kind: 'notice', text: 'Moved' },
  ];
  for (const event of valid) assert.deepEqual(parseAppEvent(event), event, event.kind);
  const invalid = [
    null,
    'email-changed',
    {},
    { kind: 'workspace-changed' },
    { kind: 'email-changed', account_id: 2 },
    { kind: 'email-changed', account_id: 2, ids: ['71'] },
    { kind: 'email-changed', account_id: -1, ids: [1] },
    { kind: 'counts-changed' },
    { kind: 'undo-changed', action: { id: 1 } },
    { kind: 'undo-removed', id: 'x' },
    { kind: 'sync-progress', account_id: 1, phase: 'paused', added: 0 },
    { kind: 'notice', text: 4 },
  ];
  for (const event of invalid) assert.equal(parseAppEvent(event), null, JSON.stringify(event));
});

test('coalescing runs one pass at a time with at most one waiting, and callers share a pass', async () => {
  let running = 0;
  let peak = 0;
  let passes = 0;
  const gates = [];
  const refresh = coalescing(async () => {
    passes++;
    running++;
    peak = Math.max(peak, running);
    await new Promise((resolve) => gates.push(resolve));
    running--;
  });
  const first = refresh();
  await Promise.resolve();
  assert.equal(passes, 1);
  // Five callers arrive while the first pass runs: they share the one pass that waits behind it.
  const waiting = [refresh(), refresh(), refresh(), refresh(), refresh()];
  await Promise.resolve();
  assert.equal(passes, 1, 'Nothing starts while a pass runs');
  gates.shift()();
  await first;
  await Promise.resolve();
  assert.equal(passes, 2, 'The waiting pass starts when the first ends');
  gates.shift()();
  await Promise.all(waiting);
  assert.equal(passes, 2, 'Five callers made one pass');
  assert.equal(peak, 1);
  const later = refresh();
  await Promise.resolve();
  assert.equal(passes, 3, 'An idle runner starts at once');
  gates.shift()();
  await later;
});

test('activity-changed parses with its ids and is not an unknown payload', () => {
  assert.deepEqual(parseAppEvent({ kind: 'activity-changed', ids: [3, 4] }), {
    kind: 'activity-changed',
    ids: [3, 4],
  });
  assert.equal(parseAppEvent({ kind: 'activity-changed', ids: ['x'] }), null);
  assert.equal(parseAppEvent({ kind: 'activity-changed' }), null);
});

test('activity-changed leaves the page and the workspace untouched', () => {
  const page = deepFreeze({ items: [], total: 0 });
  const workspace = deepFreeze({ undo_actions: [], counts: {} });
  const effects = applyEvent(
    page,
    workspace,
    { kind: 'activity-changed', ids: [1] },
    { folder: 'Inbox', account_id: null, search: '', threads: false },
  );
  assert.equal(effects.page, page);
  assert.equal(effects.workspace, workspace);
  assert.deepEqual(effects.needsRows, []);
  assert.equal(effects.reload, false);
  assert.equal(effects.snapshot, false);
});
