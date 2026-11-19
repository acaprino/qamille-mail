// The calendar page loads in two ways: a live read of every shown calendar (Google answers in seconds)
// and a refresh every 2 s that only re-reads the local calendar. The refresh must never make the
// slow live read be dropped, and must never undo a live result that landed while it was running.
import assert from 'node:assert/strict';
import test from 'node:test';
import { LoadGate } from '../src/lib/load-gate.ts';

test('a live read is shown even when 2-second refreshes started and finished while it ran', () => {
  const gate = new LoadGate();
  const live = gate.begin(false);
  for (let i = 0; i < 6; i++) {
    const refresh = gate.begin(true);
    assert.equal(gate.accept(refresh), true, 'a refresh with nothing newer is shown');
  }
  assert.equal(gate.accept(live), true, 'the slow live read still lands');
});

test('a refresh that was running when a live result landed is dropped, so it cannot undo it', () => {
  const gate = new LoadGate();
  const live = gate.begin(false);
  const refresh = gate.begin(true);
  assert.equal(gate.accept(live), true);
  assert.equal(gate.accept(refresh), false, 'it was built from the data before the live result');
  assert.equal(gate.accept(gate.begin(true)), true, 'the next refresh starts from the live data');
});

test('a refresh started before a newer live read is dropped', () => {
  const gate = new LoadGate();
  const refresh = gate.begin(true);
  gate.begin(false);
  assert.equal(gate.accept(refresh), false);
});

test('an older live read never overwrites a newer one (the user changed week or hid a calendar)', () => {
  const gate = new LoadGate();
  const older = gate.begin(false);
  const newer = gate.begin(false);
  assert.equal(gate.accept(older), false);
  assert.equal(gate.accept(newer), true);
});

test('partial results of one live read can all be shown, then its final result', () => {
  const gate = new LoadGate();
  const live = gate.begin(false);
  assert.equal(gate.accept(live), true, 'the fast calendar answered');
  assert.equal(gate.accept(live), true, 'the next one answered');
  assert.equal(gate.accept(live), true, 'the final merge');
});
