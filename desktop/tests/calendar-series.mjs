import assert from 'node:assert/strict';
import test from 'node:test';
import { calendarFormChanges, calendarTimeChanges } from '../src/lib/calendar-edit.ts';
import { configureRegional } from '../src/lib/regional.ts';
import { copiedDraft } from '../src/lib/composer.ts';

const form = { title: 'Weekly', start: '2026-04-03T09:00', end: '2026-04-03T10:00', allDay: false, location: '', description: '', attendees: 'one@example.test', recurrence: '', link: '' };
const occurrence = { start: { dateTime: '2026-04-03T09:00:00+02:00', timeZone: 'Europe/Rome' }, end: { dateTime: '2026-04-03T10:00:00+02:00', timeZone: 'Europe/Rome' } };
const master = { start: { dateTime: '2026-03-20T09:00:00+01:00', timeZone: 'Europe/Rome' }, end: { dateTime: '2026-03-20T10:00:00+01:00', timeZone: 'Europe/Rome' } };

test('title-only series edits do not change dates, attendees, recurrence or time zone', () => {
  configureRegional({ time_zone: 'Europe/Rome', locale: 'en-US' });
  const after = { ...form, title: 'Renamed' };
  assert.deepEqual(calendarFormChanges(form, after, false), { title: 'Renamed' });
  assert.deepEqual(calendarTimeChanges(form, after, occurrence, master), {});
});
test('a one-hour series edit keeps master date and civil hour across DST', () => {
  configureRegional({ time_zone: 'Europe/Rome', locale: 'en-US' });
  assert.deepEqual(calendarTimeChanges(form, { ...form, start: '2026-04-03T10:00', end: '2026-04-03T11:00' }, occurrence, master), {
    start: '2026-03-20T09:00:00.000Z', end: '2026-03-20T10:00:00.000Z', all_day: false, time_zone: 'Europe/Rome',
  });
});
test('series edits use the recurrence zone when the app displays another zone', () => {
  configureRegional({ time_zone: 'America/New_York', locale: 'en-US' });
  const before = { ...form, start: '2026-04-03T03:00', end: '2026-04-03T04:00' };
  const result = calendarTimeChanges(before, { ...before, start: '2026-04-03T04:00', end: '2026-04-03T05:00' }, occurrence, master);
  assert.equal(result.start, '2026-03-20T09:00:00.000Z');
  assert.equal(result.time_zone, 'Europe/Rome');
});
test('a pending series edit uses its captured display zone after preferences change', () => {
  configureRegional({ time_zone: 'Asia/Tokyo', locale: 'en-US' });
  const before = { ...form, start: '2026-04-03T03:00', end: '2026-04-03T04:00' };
  const result = calendarTimeChanges(before, { ...before, start: '2026-04-03T04:00', end: '2026-04-03T05:00' }, occurrence, master, 'Europe/Rome', 'America/New_York');
  assert.equal(result.start, '2026-03-20T09:00:00.000Z');
  assert.equal(result.end, '2026-03-20T10:00:00.000Z');
  assert.equal(result.time_zone, 'Europe/Rome');
});
test('all-day series shifts preserve master dates and exclusive ends', () => {
  configureRegional({ time_zone: 'Europe/Rome', locale: 'en-US' });
  const before = { ...form, allDay: true, start: '2026-04-03', end: '2026-04-04' };
  assert.deepEqual(calendarTimeChanges(before, { ...before, start: '2026-04-04', end: '2026-04-05' }, { start: { date: '2026-04-03' }, end: { date: '2026-04-04' } }, { start: { date: '2026-03-20' }, end: { date: '2026-03-21' } }), {
    start: '2026-03-21', end: '2026-03-22', time_zone: 'Europe/Rome', all_day: true,
  });
});
test('all-day conversion retains the series origin instead of the selected occurrence', () => {
  configureRegional({ time_zone: 'Europe/Rome', locale: 'en-US' });
  assert.deepEqual(calendarTimeChanges(form, { ...form, allDay: true, start: '2026-04-03', end: '2026-04-04' }, occurrence, master), {
    start: '2026-03-20', end: '2026-03-21', time_zone: 'Europe/Rome', all_day: true,
  });
});
test('a shifted nonexistent master time fails before contacting the provider', () => {
  configureRegional({ time_zone: 'Europe/Rome', locale: 'en-US' });
  const before = { ...form, start: '2026-04-05T01:30' };
  assert.throws(() => calendarTimeChanges(before, { ...before, start: '2026-04-05T02:30' }, { ...occurrence, start: { dateTime: '2026-04-05T01:30:00+02:00' } }, { ...master, start: { dateTime: '2026-03-29T01:30:00+01:00', timeZone: 'Europe/Rome' } }), /does not exist/);
});
test('recovered draft copies preserve message content without cloud identity or schedule', () => {
  const original = { id: 1, account_id: 2, to: 'to@test', cc: 'cc@test', bcc: 'bcc@test', subject: 'Local', body: '<p>Unsaved text</p>', reply_to: 3, send_at: 123, attachments: ['file.txt'], updated: 1, format: 'Html', signature_id: 4, quote: 'Quote', remote_id: 'provider-original', cloud_sync_pending: true };
  const created = { ...original, id: 5, updated: 6, remote_id: null, cloud_sync_pending: false };
  const result = copiedDraft(original, created);
  assert.equal(result.id, 5);
  assert.equal(result.updated, 6);
  assert.equal(result.remote_id, null);
  assert.equal(result.cloud_sync_pending, false);
  assert.equal(result.send_at, null);
  assert.equal(result.body, original.body);
  assert.equal(result.bcc, original.bcc);
  assert.deepEqual(result.attachments, original.attachments);
});
