// What a screen reader is given by the BUILT executable: the UI Automation tree (the API Narrator
// reads) of its window, for the thread list, the conversation rows, the arrows and the reading pane.
// A real screen reader cannot be driven from a test; UIA is the layer it reads, so names, roles,
// states and descriptions are checked there. Synthetic sample workspace only (see support/native-app.mjs).
//
// Checks D1, D3 and D4 are the accessibility rules of the conversation rows (an unread reply says so
// in words, the opened replies are a named group, the initials are not read before the name): they are
// collected, so a regression lists all of them, and make the run exit with 1 at the end.
// Artifacts: checks/a11y-native/<run>/uia-tree.json and uia-summary.txt
//   node tests/a11y-native.mjs        (from desktop/, with release/Qamille.exe or QAMILLE_NATIVE_EXE)
import { expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { IDS, SUBJECT, withConversation } from './support/conversation-fixture.mjs';
import { openAllGroups } from './support/groups.mjs';
import { launchNative, root } from './support/native-app.mjs';

const sample = JSON.parse(await fs.readFile(path.join(root, 'desktop/src/lib/sample.json'), 'utf8'));
const fixture = withConversation(sample);
for (const email of fixture.emails) email.reminder_at = null;

const app = await launchNative('a11y-native', { store: fixture, browserArgs: '--force-renderer-accessibility' });
const { page } = app;
const defects = [];
const defect = (id, ok, message) => {
  console.log(`${ok ? 'PASS' : 'FAIL'} ${id}: ${message}`);
  if (!ok) defects.push(`${id}: ${message}`);
};
try {
  await expect(page.locator('.mail-row').first()).toBeVisible();
  const modeButton = page.getByRole('button', { name: 'Threads', exact: true });
  await modeButton.click();
  await expect(modeButton).toHaveAttribute('aria-pressed', 'true');
  await openAllGroups(page); // a conversation that has been read sits in the closed Seen group
  const row = page.locator('.mail-row', { hasText: SUBJECT });
  await row.locator('.thread-toggle').click();
  await expect(page.locator('.reply-row')).toHaveCount(4);
  // Open reply A: it is "pressed"; the unread reply B stays unread so its name can be read.
  await page.locator(`.reply-row[data-reply-id="${IDS.a}"] .reply-open`).click();
  await expect(page.locator('.message-body').first()).toContainText('I can take the schedule');
  await expect(page.locator(`.reply-row[data-reply-id="${IDS.many}"]`)).toHaveClass(/unread/);
  await page.mouse.move(0, 0);

  // ---- read the UIA tree with Windows PowerShell ----
  const out = path.join(app.data, 'uia-tree.json');
  const script = path.join(import.meta.dirname, 'support/uia-dump.ps1');
  const { stdout } = await promisify(execFile)('powershell.exe',
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script, '-ProcessId', String(app.child.pid), '-Out', out],
    { windowsHide: true, timeout: 120000, maxBuffer: 1 << 24 });
  const tree = JSON.parse((await fs.readFile(out, 'utf8')).replace(/^﻿/, ''));
  const all = tree.elements;
  console.log(`UIA: window "${tree.title}", ${all.length} elements (${stdout.trim()})`);
  assert.ok(all.length > 60, 'the web content is exposed to UI Automation');

  const state = (e) => [e.toggle && `toggle ${e.toggle}`, e.expand && e.expand, e.selected != null && `selected ${e.selected}`].filter(Boolean).join(', ');
  const line = (e) => `${`${e.type}/${e.localized}`.padEnd(30)} "${e.name}"${state(e) ? `  [${state(e)}]` : ''}${e.help ? `  (help: ${e.help})` : ''}${e.focusable ? '' : '  (not focusable)'}`;
  await fs.writeFile(path.join(app.data, 'uia-summary.txt'), all.map(line).join('\n'));
  const byName = (re, type) => all.filter((e) => re.test(e.name) && (!type || e.type === type));

  // ---- the window and the landmarks ----
  assert.ok(/Qamille/i.test(tree.title), `the window has a name (${tree.title})`);
  const listRegion = byName(/^Thread list$/);
  assert.ok(listRegion.length >= 1, 'the list is a named region "Thread list" in Conversations mode');
  const pane = byName(/^Reading pane$/);
  assert.ok(pane.length >= 1, 'the reading pane is a named region');
  assert.ok([listRegion[0], pane[0]].every((e) => e.type === 'Group' && /region/i.test(e.localized)), 'both are exposed as regions (landmarks)');
  console.log(`PASS The list ("${listRegion[0].name}") and the reading pane are exposed as named regions (${listRegion[0].type}/${listRegion[0].localized})`);

  // ---- the mode switch ----
  const threadsButton = byName(/^Threads$/, 'Button')[0];
  assert.ok(threadsButton, 'the mode switch has a "Threads" button');
  assert.equal(threadsButton.toggle, 'On', 'and it says it is pressed');
  console.log('PASS The "Threads" button is a toggle that reads as On');

  // ---- the conversation row, its arrow and the replies ----
  const mainRow = all.find((e) => e.type === 'Button' && /^Open\b/.test(e.name) && /Luca Bianchi/.test(e.name) && /Launch plan/.test(e.name));
  assert.ok(mainRow, 'the main row is a button that names the sender and the subject');
  const arrow = byName(/^Conversation$/, 'Button')[0];
  assert.ok(arrow, 'the arrow is a named button');
  assert.equal(arrow.name, 'Conversation', 'one name for both states: aria-expanded carries the state');
  assert.equal(arrow.expand, 'Expanded', 'and it says it is expanded');
  assert.match(arrow.help, /5 messages in this thread/, 'and its description is the total');
  const replies = all.filter((e) => e.type === 'Button' && /^Open\b/.test(e.name) && e !== mainRow && /\bto (you|Maria Rossi|Luca Bianchi)\b/.test(e.name));
  assert.equal(replies.length, 4, `four reply rows are buttons that say who they answer (${replies.length})`);
  const pressed = replies.filter((e) => e.toggle === 'On');
  assert.equal(pressed.length, 1, 'exactly one reply is pressed: the one open in the reading pane');
  assert.match(pressed[0].name, /Luca Bianchi/, 'and it is the one that was opened (Luca\'s reply A)');
  const folds = byName(/^Replies to this message$/, 'Button');
  assert.equal(folds.length, 2, 'two replies have replies under them, each with a fold button');
  assert.ok(folds.every((e) => e.expand === 'Expanded' && e.focusable), 'the fold buttons are focusable and say they are expanded');
  console.log('PASS The main row, the arrow (expanded, with its total), four reply rows and two fold buttons have sensible names, roles and states');

  // ---- D1: the unread reply says so ----
  const unreadReply = replies.find((e) => /Maria Rossi/.test(e.name) && /\+11/.test(e.name));
  assert.ok(unreadReply, 'the unread reply (to twelve people) is in the tree');
  // The button's own name, which is what is read when it takes the focus, says ", unread" in words
  // (the dot itself is hidden from assistive technology).
  const readReply = replies.find((e) => /Luca Bianchi to Maria Rossi/.test(e.name));
  defect('D1', /unread/i.test(unreadReply.name) && !/unread/i.test(readReply.name),
    `the unread reply's own name says "Unread" and a read reply's does not. Unread: "...${unreadReply.name.slice(-45)}"; read: "...${readReply.name.slice(-30)}"`);

  // ---- D4: initials are read before the name ----
  defect('D4', !replies.some((e) => /^Open [A-Z]{1,2} /.test(e.name)) && !/^Open [A-Z]{1,2} /.test(mainRow.name),
    `the avatar initials are read before the sender (ThreadReplies.tsx, MailList.tsx). Name given: "${mainRow.name.slice(0, 70)}"`);

  // ---- D3: the opened replies are one group ----
  const group = all.find((e) => /^(Group|List)$/.test(e.type) && /repl/i.test(e.name));
  defect('D3', !!group, 'the opened replies are exposed as a named group or list (ThreadReplies.tsx is a plain div)');

  // ---- the reading pane controls ----
  for (const name of ['Reply', 'Forward', 'Archive message', 'Snooze until tomorrow', 'Done']) {
    const found = all.find((e) => e.type === 'Button' && e.name.startsWith(name));
    assert.ok(found, `the reading pane has a "${name}" button`);
  }
  const heading = all.find((e) => e.type === 'Text' && /heading|intestazione/i.test(e.localized) && e.name === 'Launch plan: who owns what?');
  assert.ok(heading, 'the message title is a heading');
  console.log('PASS The reading pane: Reply, Forward, Archive, Snooze and Done are named buttons; the title is a heading');

  // ---- every control a keyboard user can reach has a name ----
  const unnamed = all.filter((e) => e.focusable && /^(Button|CheckBox|RadioButton|Hyperlink|ComboBox|Edit|TabItem|MenuItem)$/.test(e.type) && !e.name.trim());
  assert.equal(unnamed.length, 0, `controls without a name: ${unnamed.map((e) => e.type).join(', ')}`);
  console.log(`PASS Every focusable control has a name (${all.filter((e) => e.focusable).length} focusable elements)`);

  // ---- the reader no longer repeats the conversation ----
  assert.equal(byName(/^Conversation messages$/).length, 0, 'the reading pane has no "Conversation messages" region any more');
  await fs.copyFile(path.join(app.data, 'uia-summary.txt'), path.join(path.dirname(app.data), 'uia-summary-latest.txt')).catch(() => {});
  console.log(`Artifacts: ${app.data}`);
  assert.deepEqual(app.pageErrors, [], 'no page error');
} catch (error) {
  await page.screenshot({ path: path.join(app.data, 'failure.png') }).catch(() => {});
  console.error(`artifacts: ${app.data}`);
  throw error;
} finally {
  await app.stop();
}
if (defects.length) {
  console.error(`\n${defects.length} known defect(s):\n- ${defects.join('\n- ')}`);
  process.exit(1);
}
