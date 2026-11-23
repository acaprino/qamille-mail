// Node's strict assert that tells a failure and goes on, so one run lists every assertion that differs (a design pin that Sigrid now owns,
// a behaviour that changed) instead of stopping at the first. The process exits with 1 if any failed. Drop-in for `node:assert/strict`.
import real from 'node:assert/strict';

function report(error) {
  process.exitCode = 1;
  const frame = String(error?.stack ?? '')
    .split('\n')
    .find((line) => /tests[\/][^\/]+\.mjs:\d+/.test(line) && !/soft-assert/.test(line));
  const where = frame ? frame.trim().replace(/^.*tests[\/]/, '').replace(/\)$/, '') : '?';
  console.log(`SOFT FAIL ${where} :: ${String(error?.message ?? error).split('\n').slice(0, 5).join(' | ').slice(0, 320)}`);
}

const soft = (value, message) => {
  try {
    real(value, message);
  } catch (error) {
    report(error);
  }
};
for (const name of ['ok', 'equal', 'notEqual', 'deepEqual', 'notDeepEqual', 'strictEqual', 'notStrictEqual', 'deepStrictEqual', 'notDeepStrictEqual', 'match', 'doesNotMatch']) {
  soft[name] = (...args) => {
    try {
      real[name](...args);
    } catch (error) {
      report(error);
    }
  };
}
soft.fail = real.fail;
soft.rejects = real.rejects;
soft.throws = real.throws;
export default soft;
