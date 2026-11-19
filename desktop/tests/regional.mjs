import assert from 'node:assert/strict';
import test from 'node:test';
import {
  configureRegional,
  dateInput,
  dateKey,
  firstWeekday,
  parseLocalDateTime,
  regionalTimeZone,
  regionalWarning,
  shortDate,
} from '../src/lib/regional.ts';

test('configured time zone controls displayed calendar date and input value', () => {
  configureRegional({ time_zone: 'Europe/Rome', locale: 'it-IT' });
  const timestamp = Date.parse('2026-10-15T22:30:00Z') / 1000;
  assert.equal(dateKey(timestamp), '2026-10-16');
  assert.equal(dateInput(timestamp), '2026-10-16T00:30');
  assert.equal(parseLocalDateTime('2026-10-16T00:30'), timestamp);
  configureRegional({ time_zone: 'UTC', locale: 'en-US' });
  assert.equal(dateKey(timestamp), '2026-10-15');
});

test('repeated local time chooses first instant, nonexistent local time is rejected', () => {
  configureRegional({ time_zone: 'Europe/Rome', locale: 'it-IT' });
  assert.equal(parseLocalDateTime('2026-10-25T02:30'), Date.parse('2026-10-25T00:30:00Z') / 1000);
  configureRegional({ time_zone: 'America/Havana', locale: 'en-US' });
  assert.throws(() => parseLocalDateTime('2026-03-08T00:30'), /does not exist/);
  assert.equal(parseLocalDateTime('2026-03-08T01:00'), Date.parse('2026-03-08T05:00:00Z') / 1000);
});

test('unknown zone is declared rather than using an unrelated system zone', () => {
  configureRegional({ time_zone: 'Invalid/Zone', locale: 'it-IT' });
  assert.equal(regionalTimeZone(), 'UTC');
  assert.match(regionalWarning(), /UTC/);
});

test('regional calendar week starts follow locale', () => {
  configureRegional({ time_zone: 'UTC', locale: 'it-IT' });
  assert.equal(firstWeekday(), 1);
  configureRegional({ time_zone: 'UTC', locale: 'en-US' });
  assert.equal(firstWeekday(), 0);
});

test('a refresh does not rebuild the Intl formatters (ADR-0007)', () => {
  configureRegional({ time_zone: 'Europe/Rome', locale: 'it-IT' });
  const timestamp = Date.parse('2026-10-15T22:30:00Z') / 1000;
  const recent = Date.now() / 1000 - 3600;
  const first = [shortDate(timestamp), shortDate(recent)];
  const Real = Intl.DateTimeFormat;
  let built = 0;
  Intl.DateTimeFormat = new Proxy(Real, {
    construct(target, args) {
      built += 1;
      return new target(...args);
    },
  });
  try {
    const again = [shortDate(timestamp), shortDate(recent)];
    for (let i = 0; i < 50; i++) shortDate(timestamp);
    assert.deepEqual(again, first, 'the text does not change');
    assert.equal(built, 0, 'repeated shortDate calls build no new formatter');
    configureRegional({ time_zone: 'UTC', locale: 'en-US' });
    assert.notEqual(shortDate(timestamp), first[0], 'a changed locale or zone still formats differently');
  } finally {
    Intl.DateTimeFormat = Real;
  }
});

