// Task 2.4 (ADR-0007 v3 decision 7, INV-009): the single activity indicator.
// Unit tests for the pure reducers in src/lib/activity.ts, plus Playwright steps
// on the browser preview (set QAMILLE_PREVIEW_URL; without it they are skipped).
import assert from 'node:assert/strict';
import test from 'node:test';
import { chromium, expect as pw } from '@playwright/test';
import {
  activityHideDelayMs,
  activityStaleMs,
  activityMinVisibleMs,
  applyActivityEvent,
  displayableActivities,
  isStale,
  primaryActivity,
  withConfirming,
} from '../src/lib/activity.ts';
import { applyEvent, parseAppEvent as parseAppEventForTest } from '../src/lib/events.ts';
import { openAllGroups } from './support/groups.mjs';

const deepFreeze = (value) => {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    Object.values(value).forEach(deepFreeze);
  }
  return value;
};
const ids = (list) => list.map((activity) => activity.id);
const syncing = (account_id, added = 0) => ({
  kind: 'sync-progress',
  account_id,
  phase: 'syncing',
  added,
});
const syncDone = (account_id, phase = 'complete') => ({
  kind: 'sync-progress',
  account_id,
  phase,
  added: 0,
});
const indexAt = (account_id, percent) => ({ kind: 'index-progress', account_id, percent });
const model = (percent, done = false) => ({ kind: 'model-progress', percent, done });
const contacts = (running, account_id = null) => ({ kind: 'contacts-sync', account_id, running });
const undoAction = (id, overrides = {}) => ({
  id,
  operation: { kind: 'mail', email_id: id, action: 'markAsDone', value: null },
  subject: `Message ${id}`,
  created_at_ms: 0,
  expires_at_ms: 5000,
  status: 'pending',
  error: null,
  ...overrides,
});
const workspaceOf = (actions = []) => ({ undo_actions: actions, accounts: [], counts: {} });
const emptyPage = () => ({ items: [], total: 0, next_offset: null });
const filter = { folder: 'Inbox', account_id: null, search: '', threads: false };

test('SyncProgress syncing adds the account activity; the closing phase removes it', () => {
  const started = applyActivityEvent(deepFreeze([]), syncing(2), 1000);
  assert.deepEqual(ids(started), ['sync:2']);
  assert.deepEqual(started[0], {
    id: 'sync:2',
    kind: 'sync',
    accountId: 2,
    phase: 'syncing',
    percent: null,
    since: 1000,
    last: 1000,
  });
  // A following event keeps the start and moves the last-seen instant.
  const moved = applyActivityEvent(started, syncing(2, 5), 4000);
  assert.equal(moved[0].since, 1000);
  assert.equal(moved[0].last, 4000);
  assert.deepEqual(ids(applyActivityEvent(moved, syncDone(2), 5000)), []);
  assert.deepEqual(ids(applyActivityEvent(moved, syncDone(2, 'error'), 5000)), []);
  // Another account syncs on its own row.
  const both = applyActivityEvent(moved, syncing(3), 4200);
  assert.deepEqual(ids(both), ['sync:2', 'sync:3']);
  assert.deepEqual(ids(applyActivityEvent(both, syncDone(2), 4300)), ['sync:3']);
});

test('IndexProgress creates and updates the account activity; 100% ends it', () => {
  const first = applyActivityEvent(deepFreeze([]), indexAt(2, 40), 1000);
  assert.deepEqual(first[0], {
    id: 'index:2',
    kind: 'index',
    accountId: 2,
    phase: 'indexing',
    percent: 40,
    since: 1000,
    last: 1000,
  });
  const moved = applyActivityEvent(first, indexAt(2, 55), 2000);
  assert.equal(moved[0].percent, 55);
  assert.equal(moved[0].since, 1000, 'the start does not move');
  assert.equal(moved[0].last, 2000);
  assert.deepEqual(ids(applyActivityEvent(moved, indexAt(2, 100), 3000)), []);
});

test('HistoryProgress keeps one account activity with counts, no percent; complete ends it', () => {
  const event = (fetched, complete = false) => ({ kind: 'history-progress', account_id: 3, fetched, total: 48300, complete });
  const first = applyActivityEvent([], event(500), 1000);
  assert.deepEqual(ids(first), ['history:3']);
  assert.equal(first[0].percent, null);
  assert.equal(first[0].phase, '500/48300');
  const second = applyActivityEvent(first, event(1000), 2000);
  assert.equal(second[0].since, 1000, 'the start never moves');
  assert.equal(second[0].phase, '1000/48300');
  assert.deepEqual(applyActivityEvent(second, event(48300, true), 3000), []);
});

test('ModelProgress adds the single model activity; done removes it', () => {
  const started = applyActivityEvent(deepFreeze([]), model(null), 1000);
  assert.deepEqual(ids(started), ['model']);
  assert.equal(started[0].percent, null);
  const moved = applyActivityEvent(started, model(42), 2000);
  assert.equal(moved[0].percent, 42);
  assert.equal(moved[0].since, 1000);
  assert.deepEqual(ids(applyActivityEvent(moved, model(100, true), 3000)), []);
  // A failed install also ends the activity: errors surface elsewhere, never here.
  assert.deepEqual(ids(applyActivityEvent(moved, model(null, true), 3000)), []);
});

test('ContactsSync running adds the single contacts activity; the end removes it', () => {
  const started = applyActivityEvent(deepFreeze([]), contacts(true), 1000);
  assert.deepEqual(ids(started), ['contacts']);
  assert.equal(started[0].accountId, null);
  // Per-book announcements move the same activity to the account being read.
  const moved = applyActivityEvent(started, contacts(true, 2), 2000);
  assert.deepEqual(ids(moved), ['contacts']);
  assert.equal(moved[0].accountId, 2);
  assert.equal(moved[0].since, 1000, 'a per-book event is not a restart');
  assert.deepEqual(ids(applyActivityEvent(moved, contacts(false), 3000)), []);
});

test('events of other kinds leave the registry untouched (same object)', () => {
  const list = deepFreeze(applyActivityEvent([], syncing(2), 1000));
  for (const event of [
    { kind: 'email-changed', account_id: 2, ids: [1] },
    { kind: 'counts-changed', account_id: 2 },
    { kind: 'notice', text: 'hello' },
    { kind: 'drafts-changed', ids: [] },
    { kind: 'undo-removed', id: 1 },
  ])
    assert.equal(applyActivityEvent(list, event, 2000), list, `${event.kind} changes nothing`);
});

test('an undo action past its window is counted as confirming, with a stable start', () => {
  // Inside the window the action is UndoStack's: the indicator does not count it.
  assert.deepEqual(withConfirming([], [undoAction(1)], 4999), []);
  // The window closes (or the event says `committing`): the count appears.
  const committing = undoAction(1, { status: 'committing' });
  const counted = withConfirming([], [committing], 6000);
  assert.deepEqual(ids(counted), ['confirming']);
  assert.deepEqual(counted[0], {
    id: 'confirming',
    kind: 'confirming',
    accountId: null,
    phase: '1',
    percent: null,
    since: 6000,
    last: 6000,
  });
  // A second action joins: the count moves, the start does not.
  const two = withConfirming(counted, [committing, undoAction(2)], 7000);
  assert.equal(two[0].phase, '2');
  assert.equal(two[0].since, 6000);
  // Confirmed: the count leaves.
  assert.deepEqual(withConfirming(two, [], 8000), []);
  // A failed action is a notice of its own, never a confirmation in progress.
  assert.deepEqual(withConfirming([], [undoAction(1, { status: 'failed', error: 'x' })], 6000), []);
  // A retrying confirmation keeps its own row in the stack: not counted here.
  assert.deepEqual(
    withConfirming([], [undoAction(1, { attempts: 1, retry_at_ms: 20_000 })], 6000),
    [],
  );
});

test('withConfirming is stable: an unchanged count returns the very same list (safe on every tick)', () => {
  const committing = undoAction(1, { status: 'committing' });
  const counted = withConfirming([], [committing], 6000);
  const again = withConfirming(counted, [committing], 60_000);
  assert.equal(again, counted, 'the entry is not rebuilt while the count stands');
  assert.equal(again[0].since, 6000);
  assert.equal(again[0].last, 6000, 'the stale wait runs from the last real change');
  const empty = deepFreeze([]);
  assert.equal(withConfirming(empty, [], 6000), empty, 'no actions, no rebuild');
});

test('the confirming count follows the workspace the undo events build', () => {
  // The event path of Task 2.2/2.3: undo-changed updates workspace.undo_actions,
  // and the indicator derives its confirming count from that list.
  let workspace = deepFreeze(workspaceOf());
  const committing = undoAction(9, { status: 'committing', expires_at_ms: 1000 });
  workspace = applyEvent(emptyPage(), workspace, { kind: 'undo-changed', action: committing }, filter)
    .workspace;
  const counted = withConfirming([], workspace.undo_actions, 5000);
  assert.deepEqual(ids(counted), ['confirming'], 'UndoChanged with committing creates the count');
  assert.equal(counted[0].phase, '1');
  const removed = applyEvent(emptyPage(), workspace, { kind: 'undo-removed', id: 9 }, filter);
  assert.deepEqual(withConfirming(counted, removed.workspace.undo_actions, 6000), []);
});

test('primaryActivity follows confirming > sync > model > index > contacts', () => {
  const at = (kind, id, since = 0) => ({
    id,
    kind,
    accountId: null,
    phase: kind,
    percent: null,
    since,
    last: since,
  });
  assert.equal(primaryActivity([]), null);
  const at5 = [
    at('contacts', 'contacts'),
    at('index', 'index:1'),
    at('model', 'model'),
    at('sync', 'sync:2'),
    at('confirming', 'confirming'),
  ];
  assert.equal(primaryActivity(at5).kind, 'confirming');
  assert.equal(
    primaryActivity(at5.filter((activity) => activity.kind !== 'confirming')).kind,
    'sync',
  );
  assert.equal(primaryActivity(at5.slice(0, 3)).kind, 'model');
  assert.equal(primaryActivity(at5.slice(0, 2)).kind, 'index');
  assert.equal(primaryActivity(at5.slice(0, 1)).kind, 'contacts');
  // One kind, several accounts: the one that started first leads.
  assert.equal(primaryActivity([at('sync', 'sync:2', 500), at('sync', 'sync:1', 100)]).id, 'sync:1');
});

test('an activity is displayable only after 400 ms (a 200 ms one never shows)', () => {
  const list = applyActivityEvent([], syncing(2), 1000);
  assert.deepEqual(displayableActivities(list, 1000 + activityMinVisibleMs - 1), []);
  assert.deepEqual(ids(displayableActivities(list, 1000 + activityMinVisibleMs)), ['sync:2']);
  // The brief's boundary: an activity that lives 200 ms is never displayable.
  const gone = applyActivityEvent(list, syncDone(2), 1200);
  for (const now of [1000, 1100, 1200, 1300, 2000])
    assert.deepEqual(displayableActivities(now < 1200 ? list : gone, now), []);
  assert.equal(activityMinVisibleMs, 400);
  assert.equal(activityHideDelayMs, 1000);
});

test('an activity without events for 5 min is stale, and a new event clears it', () => {
  const list = applyActivityEvent([], syncing(2), 1000);
  assert.equal(isStale(list[0], 1000 + activityStaleMs), false, 'exactly the threshold is not stale yet');
  assert.equal(isStale(list[0], 1000 + activityStaleMs + 1), true);
  const moved = applyActivityEvent(list, syncing(2), 1000 + activityStaleMs + 1);
  assert.equal(isStale(moved[0], 1000 + activityStaleMs + 1), false, 'an event resets the wait');
  assert.equal(activityStaleMs, 300_000);
});

test('an index-progress at the same percent is a heartbeat: it moves "last", never "since"', () => {
  // The backend repeats an unchanged percent after 30 s of work (INDEX_HEARTBEAT_SECS, indexing.rs).
  const heartbeatMs = 30_000;
  const started = applyActivityEvent([], indexAt(2, 0), 1000);
  const beat = applyActivityEvent(started, indexAt(2, 0), 1000 + heartbeatMs);
  assert.equal(beat[0].since, 1000, 'the start never moves');
  assert.equal(beat[0].last, 1000 + heartbeatMs, 'the heartbeat resets the stale wait');
  assert.equal(isStale(beat[0], 1000 + activityStaleMs + 1), false, 'kept alive by the heartbeat');
  assert.equal(isStale(beat[0], 1000 + heartbeatMs + activityStaleMs + 1), true, 'without the next one it goes stale');
});

// ---------------------------------------------------------------------------
// Browser preview (Playwright). Skipped without QAMILLE_PREVIEW_URL.
// ---------------------------------------------------------------------------
const previewUrl = process.env.QAMILLE_PREVIEW_URL;
const skip = !previewUrl ? { skip: 'set QAMILLE_PREVIEW_URL to run the preview steps' } : {};

async function withPage(fakeClock, run) {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const context = await browser.newContext({
    locale: 'en-US',
    viewport: { width: 1440, height: 900 },
  });
  const page = await context.newPage();
  const errors = [];
  const warnings = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'warning') warnings.push(message.text());
  });
  page.setDefaultTimeout(15000);
  if (fakeClock) await page.clock.install();
  try {
    await page.goto(previewUrl);
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    await run(page, warnings);
    assert.deepEqual(errors, [], `No page error: ${errors}`);
  } finally {
    await browser.close();
  }
}
const announce = (page, detail) => page.evaluate((d) => window.__previewAnnounce(d), detail);
const indicator = (page) => page.locator('.activity-indicator');
const absent = async (page, message) => assert.equal(await indicator(page).count(), 0, message);
// Waits until the preview backend has not been called for 300 ms (at most 5 s).
const quiet = (page) =>
  page.evaluate(async () => {
    let last = window.__previewCalls.length;
    let since = Date.now();
    while (Date.now() - since < 5000) {
      await new Promise((resolve) => setTimeout(resolve, 50));
      const now = window.__previewCalls.length;
      if (now !== last) [last, since] = [now, Date.now()];
      else if (Date.now() - since >= 300) return;
    }
  });

test(
  'preview: hidden when idle; a simulated sync shows it and idle hides it within 1.5 s',
  skip,
  async () =>
    withPage(false, async (page) => {
      await quiet(page);
      await absent(page, 'the indicator is absent without work');
      await announce(page, syncing(2));
      const line = indicator(page).locator('.activity-line');
      await pw(line).toBeVisible({ timeout: 1500 });
      await pw(line).toContainText('Syncing');
      // A second, concurrent activity: the counter appears.
      await announce(page, indexAt(2, 40));
      await pw(indicator(page).locator('.activity-more')).toHaveText('+1');
      // One click opens the list: one row per activity — account, phase, percent, duration.
      await line.click();
      const rows = page.locator('.activity-panel [data-activity-id]');
      await pw(rows).toHaveCount(2);
      await pw(rows.nth(0)).toContainText('Personal');
      await pw(rows.nth(0)).toContainText('Syncing');
      await pw(rows.nth(1)).toContainText('Indexing 40%');
      await pw(rows.nth(1)).toContainText('Personal');
      // Both finish: the indicator is gone within 1.5 s (1 s hide delay), without flashing.
      await announce(page, syncDone(2));
      await announce(page, indexAt(2, 100));
      await pw(indicator(page)).toHaveCount(0, { timeout: 1500 });
      await absent(page, 'idle again: nothing on screen');
      // The panel is a history panel too: it stays open, its live section is gone.
      await pw(page.locator('.activity-panel [data-activity-id]')).toHaveCount(0);
    }),
);

test('preview: an activity shorter than 400 ms is never shown', skip, async () =>
  withPage(false, async (page) => {
    await quiet(page);
    await announce(page, syncing(2));
    await page.waitForTimeout(200);
    await announce(page, syncDone(2));
    // The whole life of the activity plus the hide delay: nothing ever appears.
    for (let i = 0; i < 10; i++) {
      await absent(page, 'a 200 ms activity does not flash');
      await page.waitForTimeout(150);
    }
  }));

test('preview: a silent activity dims with no word; a heartbeat revives it', skip, async () =>
  withPage(true, async (page) => {
    await announce(page, indexAt(2, 0));
    await page.clock.fastForward(500);
    const line = indicator(page).locator('.activity-line');
    const icon = line.locator('.activity-icon');
    await pw(line).toBeVisible();
    await pw(icon).not.toHaveClass(/quiet/);
    // The sidebar is no place for motion: nothing in the card turns, quiet or not.
    assert.equal(await line.locator('.spinning').count(), 0, 'the card has no spinner');
    // Five minutes and a second without one event: the icon dims. The signal is not motion
    // alone (prefers-reduced-motion) and shows no word: a word reads as an error.
    await page.clock.fastForward(activityStaleMs + 1000);
    await pw(icon).toHaveClass(/quiet/);
    assert.doesNotMatch(await indicator(page).innerText(), /ferma|stalled|stopped/i, 'no word on screen');
    // A screen reader hears the same fact, as hidden text of the status.
    await pw(line.getByRole('status')).toContainText('no update for several minutes');
    // The panel: the row dims too, still with no word.
    await line.click();
    const row = page.locator('.activity-panel [data-activity-id="index:2"]');
    await pw(row).toHaveClass(/quiet/);
    assert.doesNotMatch(await row.innerText(), /ferma|stalled|stopped/i, 'no word in the row');
    // The backend's heartbeat (the same percent again) revives it.
    await announce(page, indexAt(2, 0));
    await pw(icon).not.toHaveClass(/quiet/);
    await pw(row).not.toHaveClass(/quiet/);
  }));

test('preview: the ring follows the percent; only the open panel spins, and only without a percent', skip, async () =>
  withPage(false, async (page) => {
    await quiet(page);
    await announce(page, indexAt(2, 40));
    const line = indicator(page).locator('.activity-line');
    await pw(line).toBeVisible({ timeout: 1500 });
    const dash = await line.locator('.activity-ring .value').getAttribute('stroke-dasharray');
    const [share, whole] = dash.split(' ').map(Number);
    assert.ok(Math.abs(share / whole - 0.4) < 0.01, `a 40% ring covers 40% of the circle: ${dash}`);
    // A sync has no percent: the ring is only its track, and the card still does not move.
    await announce(page, syncing(2));
    await announce(page, indexAt(2, 100));
    await pw(line.locator('.activity-ring .value')).toHaveCount(0);
    assert.equal(await line.locator('.spinning').count(), 0);
    await line.click();
    const glyph = page.locator('.activity-panel [data-activity-id="sync:2"] .activity-glyph');
    await pw(glyph).toHaveClass(/spinning/);
    await announce(page, syncDone(2));
  }));

test(
  'preview: with a composer open, the card stays in the sidebar and the dock controls stay clickable',
  skip,
  async () =>
    withPage(false, async (page) => {
      await quiet(page);
      await page.getByRole('button', { name: /^New message/ }).click();
      const dock = page.locator('.composer-dock');
      await pw(dock).toBeVisible();
      const tab = dock.getByRole('tab').first();
      const close = dock.locator('.tab-close').first();
      await pw(close).toBeVisible();
      await announce(page, syncing(2));
      const line = indicator(page).locator('.activity-line');
      await pw(line).toBeVisible({ timeout: 1500 });
      assert.equal(
        await page.locator('.sidebar .activity-indicator').count(),
        1,
        'the card is part of the sidebar footer',
      );
      for (const [name, control] of [
        ['tab', tab],
        ['close', close],
      ]) {
        const hit = await control.evaluate((element) => {
          const box = element.getBoundingClientRect();
          const top = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
          return {
            control: element === top || element.contains(top),
            indicator: !!top && !!top.closest('.activity-indicator'),
          };
        });
        assert.ok(
          hit.control && !hit.indicator,
          `the dock ${name} receives the click: ${JSON.stringify(hit)}`,
        );
      }
      await announce(page, syncDone(2));
      await pw(indicator(page)).toHaveCount(0, { timeout: 1500 });
    }),
);

test('preview: nothing fixed sits over the mail but the undo notices and the toaster', skip, async () =>
  withPage(false, async (page) => {
    await quiet(page);
    await announce(page, syncing(2));
    await pw(indicator(page).locator('.activity-line')).toBeVisible({ timeout: 1500 });
    await page.locator('.activity-line').click();
    const fixed = await page.evaluate(() =>
      [...document.querySelectorAll('body *')]
        .filter((element) => getComputedStyle(element).position === 'fixed')
        .filter((element) => element.getBoundingClientRect().width > 0)
        .map((element) => element.className?.toString() || element.tagName),
    );
    assert.deepEqual(
      fixed.filter((name) => /activity/.test(name)),
      [],
      `no activity element is fixed over the page: ${fixed}`,
    );
    // The panel of an expanded sidebar lives inside it, in its flow, not over the list.
    assert.equal(await page.locator('.sidebar .activity-panel').count(), 1);
    await announce(page, syncDone(2));
  }));

test('preview: the history button opens a panel with three recent rows and a way to the full page', skip, async () =>
  withPage(false, async (page) => {
    await quiet(page);
    const toggle = page.locator('.activity-toggle');
    await pw(toggle).toBeVisible();
    // The preview history holds a failed send nobody has seen: the dot is on.
    await pw(toggle).toHaveClass(/has-unseen/);
    await toggle.click();
    const panel = page.locator('.activity-panel');
    await pw(panel).toBeVisible();
    await pw(panel.locator('.activity-recent')).toHaveCount(3);
    await pw(panel.getByRole('button', { name: 'See all activity' })).toBeVisible();
    // Opening is the "seen": the dot goes out and stays out.
    await pw(toggle).not.toHaveClass(/has-unseen/);
    await pw(toggle).toHaveAttribute('aria-expanded', 'true');
    // The newest row first.
    await pw(panel.locator('.activity-recent').first()).toContainText('Marked as done');
    // Escape and a click outside close it; so does a second click on the button.
    await page.keyboard.press('Escape');
    await pw(panel).toHaveCount(0);
    await toggle.click();
    await pw(panel).toBeVisible();
    await page.locator('.main-shell').click({ position: { x: 400, y: 300 } });
    await pw(panel).toHaveCount(0);
    await toggle.click();
    await toggle.click();
    await pw(panel).toHaveCount(0);
  }));

test(
  'preview: confirming actions are counted by the indicator, with no duplicate line in the stack',
  skip,
  async () =>
    withPage(false, async (page) => {
      const subject = 'Q4 proposal: can you review by Thursday?';
      const notices = page.getByRole('region', { name: 'Recent email actions' });
      const message = page.locator('.mail-open', { hasText: subject });
      await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
      // The message opened at startup is read, so it sits in the closed "Seen" group.
      await openAllGroups(page);
      await message.click();
      await quiet(page);
      // Keep the action `committing` after its window closes, like a sync holding the mailbox.
      await page.evaluate(() => (window.__previewCommitHold = 3000));
      await page.getByRole('button', { name: 'Done', exact: true }).click();
      await pw(notices.getByRole('button', { name: /^Undo:/ })).toBeVisible();
      // The window closes: the stack line is gone, the indicator counts the confirmation.
      await pw(notices).toHaveCount(0, { timeout: 10_000 });
      await pw(indicator(page).locator('.activity-line')).toContainText('Confirming 1 action', {
        timeout: 2000,
      });
      assert.equal(
        await page.locator('.undo-confirming').count(),
        0,
        'the old "Saving…" line no longer exists',
      );
      const occurrences = await page
        .locator('body')
        .evaluate((body) => (body.textContent.match(/Confirming 1 action/g) || []).length);
      assert.equal(occurrences, 1, 'the confirmation is counted once, in one place');
      // The commit ends: the indicator hides (1 s delay) and nothing is left.
      await pw(indicator(page)).toHaveCount(0, { timeout: 8000 });
    }),
);

// Task 2.5 (INV-009): a sync has exactly one persistent representation — the indicator.
// The toolbar sync button keeps no spinner and stays clickable; the mail list shows no phase.
test(
  'preview: a sync lives only in the indicator; the toolbar button and the list stay quiet',
  skip,
  async () =>
    withPage(false, async (page) => {
      await quiet(page);
      // Hold the manual sync request open so a persistent duplicate would be visible.
      await page.evaluate(() => (window.__previewCommandDelay = { sync_mail: 4000 }));
      const sync = page.getByRole('button', { name: 'Sync mail', exact: true });
      await sync.click();
      await page.waitForFunction(() => window.__previewCalls.includes('sync_mail'));
      // The backend sync-progress events (Task 2.2) drive the indicator and nothing else.
      await announce(page, syncing(2));
      await pw(indicator(page).locator('.activity-line')).toContainText('Syncing', {
        timeout: 1500,
      });
      assert.equal(
        await page.locator('.topbar-actions .spinning').count(),
        0,
        'the sync button carries no spinner while a sync runs',
      );
      assert.equal(
        await sync.isDisabled(),
        false,
        'the sync button stays clickable during a sync',
      );
      // MailList exposes no sync phase: no busy flag, no phase text.
      const list = page.locator('.mail-list');
      assert.notEqual(
        await list.getAttribute('aria-busy'),
        'true',
        'the mail list is not marked busy by a sync',
      );
      await pw(list).not.toContainText('Sync in progress');
      // The closing event: everything is quiet again (the indicator hides after 1 s).
      await announce(page, syncDone(2));
      await pw(indicator(page)).toHaveCount(0, { timeout: 1500 });
      assert.equal(await page.locator('.topbar-actions .spinning').count(), 0);
      assert.notEqual(await list.getAttribute('aria-busy'), 'true');
      await absent(page, 'after the closing event nothing is left on screen');
    }),
);

test(
  'preview: a bogus app-event kind warns, reloads once, and never reaches the indicator',
  skip,
  async () =>
    withPage(false, async (page, warnings) => {
      await quiet(page);
      const mark = await page.evaluate(() => window.__previewCalls.length);
      await announce(page, { kind: 'totally-bogus-event', anything: true });
      // The 2.3 forward risk: an unknown payload only warns and triggers the coalesced refresh.
      await page.waitForFunction(
        (before) => window.__previewCalls.slice(before).includes('list_mail'),
        mark,
        { timeout: 5000 },
      );
      await quiet(page);
      assert.ok(
        warnings.some((text) => text.includes('app-event not understood')),
        `the drift is reported: ${warnings}`,
      );
      const calls = await page.evaluate((before) => window.__previewCalls.slice(before), mark);
      assert.ok(calls.includes('list_mail'), `the coalesced refresh ran: ${calls}`);
      assert.equal(
        calls.filter((name) => name === 'read_rows').length,
        0,
        'nothing was patched row by row',
      );
      await absent(page, 'a bogus payload creates no activity');
    }),
);

test('a pause of the compute server is the state of the indexing activity (INV-6)', () => {
  let list = applyActivityEvent([], indexAt(1, 40), 0);
  list = applyActivityEvent(list, { kind: 'index-waiting', account_id: 1, waiting: true }, 10);
  assert.deepEqual(ids(list), ['index:1']);
  assert.equal(list[0].phase, 'waiting');
  assert.equal(list[0].percent, 40, 'the percent stays');
  list = applyActivityEvent(list, indexAt(1, 45), 20);
  assert.equal(list[0].phase, 'indexing', 'a moving percent ends the pause');
  const none = [];
  assert.equal(applyActivityEvent(none, { kind: 'index-waiting', account_id: 2, waiting: false }, 0), none, 'nothing to end');
  assert.deepEqual(parseAppEventForTest({ kind: 'index-waiting', account_id: 1, waiting: true }), { kind: 'index-waiting', account_id: 1, waiting: true });
  assert.equal(parseAppEventForTest({ kind: 'index-waiting', account_id: 1, waiting: 'yes' }), null);
});
