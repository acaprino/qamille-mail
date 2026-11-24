// The words Sigrid says by itself, in Italian and English (src/ui/words.ts): every word Sigrid has is written in both
// languages, an Italian word is not the English one left behind, the counted words agree with 1 and with 3, and a word
// that a Sigrid release adds fails here until it is translated.
//   node --test tests/sigrid-words.mjs        (from desktop/; needs `npm run sigrid:sync`)
// Sigrid's own words are read from the synced copy (vendor/sigrid/dist/index.js), not through the Vite alias, which
// plain Node does not know.
import assert from 'node:assert/strict';
import path from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';
import { qamilleWords } from '../src/ui/words.ts';

const desktop = path.resolve(import.meta.dirname, '..');
const { DEFAULT_WORDS, COLOR_NAMES } = await import(pathToFileURL(path.join(desktop, 'vendor', 'sigrid', 'dist', 'index.js')).href);

const ENGLISH = qamilleWords(false);
const ITALIAN = qamilleWords(true);

// What each function word is called with, by path: a list of argument lists. A new function word of Sigrid has no
// entry here and fails «every function word has a sample», so nobody forgets to look at it.
const SAMPLES = {
  'common.closeNotice': [['Salvato']],
  'graph.holds': [['Ordini', 3]],
  'graph.inGroup': [['Ordini']],
  'graph.moved': [['Ordini', 120, 40]],
  'graph.moreSteps': [[2]],
  'common.keysTogether': [[['Ctrl', 'K']]],
  'common.keysInSequence': [[['G', 'I']]],
  'colors.numbered': [[1], [18]],
  'date.nextDays': [[3]],
  'date.previous': [['month'], ['year'], ['decade']],
  'date.next': [['month'], ['year'], ['decade']],
  'duration.short': [[1, 30], [2, 0], [0, 5]],
  'labels.remove': [['lavoro']],
  'labels.added': [['lavoro']],
  'labels.pasted': [[1], [3]],
  'labels.removed': [['lavoro']],
  'labels.selected': [['lavoro']],
  'labels.count': [[1], [3]],
  'attachments.count': [[1], [3]],
  'attachments.openNamed': [['preventivo.pdf']],
  'attachments.removeNamed': [['preventivo.pdf']],
  'attachments.hint': [['25 MB']],
  'attachments.overLimit': [['25 MB']],
  'attachments.fileOverLimit': [['preventivo.pdf', '25 MB']],
  'attachments.filesOverLimit': [[3, '25 MB']],
  'viewer.page': [[2, 5]],
  'steps.summary': [[2, 4]],
  'progress.value': [[45, 100]],
  'list.more': [[2]],
  'table.sortBy': [['Dimensione']],
  'graph.open': [['Lavoro']],
  'graph.close': [['Lavoro']],
  'graph.closed': [['Lavoro', 3]],
  'graph.add': [['Lavoro']],
  'graph.addAfter': [['Lavoro']],
  'graph.hides': [['Ordini', 1], ['Ordini', 12]],
  'graph.chosenToo': [['Registro']],
  'graph.leadsTo': [[['Lavoro']], [['Lavoro', 'Casa', 'Posta']]],
  'graph.comesFrom': [[['Lavoro']], [['Lavoro', 'Casa', 'Posta']]],
  'graph.onPath': [['Copia', [2]], ['Copia', [1, 2, 3]]],
  'graph.line': [['Lavoro', 'to'], ['Lavoro', 'from', 'ordine'], ['Lavoro', 'with']],
  'reorder.handle': [['Posta in arrivo']],
  'reorder.taken': [['Posta in arrivo', 2, 5]],
  'reorder.moved': [['Posta in arrivo', 3, 5]],
  'reorder.put': [['Posta in arrivo', 3, 5]],
  'reorder.back': [['Posta in arrivo']],
};

// Words that are the same in both languages, by path. A word is listed here on purpose, with the reason.
const SAME = new Set([
  'graph.moreSteps', // a sign and a number: «+2»
  'common.keysTogether', // «Ctrl + K»: a key combination reads the same
  'progress.value', // «45%»: a percentage reads the same
  'duration.short', // «1h 30m»: the same abbreviations
  'kinds.file', // «file» is the Italian word too
  'kinds.email',
  'kinds.link',
  'attachments.link',
  'viewer.zoom',
  'keys.ctrl', // the name of the key on the keyboard, in both languages
  'keys.cmd',
  'keys.alt',
  'colors.names.#9bb53a', // Lime
  'colors.names.#c64fb5', // Magenta
]);

/** Every path of `value`: strings and functions are leaves, objects are walked. */
function leaves(value, prefix = '') {
  const found = [];
  for (const [key, inner] of Object.entries(value)) {
    const at = prefix ? `${prefix}.${key}` : key;
    if (inner && typeof inner === 'object') found.push(...leaves(inner, at));
    else found.push([at, inner]);
  }
  return found;
}

const get = (value, at) => at.split('.').reduce((inside, key) => inside?.[key], value);

/** What is wrong with `words` next to Sigrid's own `reference`: missing or extra paths, a different kind of word. */
export function problems(reference, words) {
  const wanted = new Map(leaves(reference));
  const given = new Map(leaves(words));
  const found = [];
  for (const [at, leaf] of wanted) {
    if (!given.has(at)) found.push(`${at} is missing`);
    else if (typeof given.get(at) !== typeof leaf) found.push(`${at} is a ${typeof given.get(at)}, Sigrid's is a ${typeof leaf}`);
  }
  for (const at of given.keys()) if (!wanted.has(at)) found.push(`${at} is not a word of Sigrid`);
  return found;
}

/** The text of a leaf: the word itself, or what a function word says for each sample. */
function said(words, at) {
  const leaf = get(words, at);
  if (typeof leaf !== 'function') return [leaf];
  return (SAMPLES[at] ?? []).map((args) => leaf(...args));
}

for (const [name, words] of [['English', ENGLISH], ['Italian', ITALIAN]]) {
  test(`every Sigrid word is written in ${name}`, () => {
    assert.deepEqual(problems(DEFAULT_WORDS, words), []);
  });
}

test('the palette has a name for each of Sigrid’s eighteen colours', () => {
  assert.equal(Object.keys(COLOR_NAMES).length, 18);
  for (const words of [ENGLISH, ITALIAN]) assert.deepEqual(Object.keys(words.colors.names).sort(), Object.keys(COLOR_NAMES).sort());
});

test('English is Sigrid’s own English', () => {
  for (const [at] of leaves(DEFAULT_WORDS)) {
    assert.deepEqual(said(ENGLISH, at), said(DEFAULT_WORDS, at), at);
  }
});

test('every function word has a sample, and says a whole sentence in both languages', () => {
  for (const [at, leaf] of leaves(DEFAULT_WORDS)) {
    if (typeof leaf !== 'function') continue;
    assert.ok(SAMPLES[at]?.length, `${at}: add a sample to SAMPLES`);
    for (const words of [ENGLISH, ITALIAN]) {
      for (const text of said(words, at)) {
        assert.equal(typeof text, 'string', at);
        assert.ok(text.trim().length > 0, `${at} says nothing`);
        assert.ok(!/undefined|NaN|\[object/.test(text), `${at} says «${text}»`);
      }
    }
  }
});

test('every word is a non-empty text', () => {
  for (const words of [ENGLISH, ITALIAN]) {
    for (const [at, leaf] of leaves(words)) {
      if (typeof leaf === 'string') assert.ok(leaf.trim().length > 0 || at.startsWith('checklist'), `${at} is empty`);
    }
  }
});

test('Italian words differ from English ones', () => {
  for (const [at] of leaves(DEFAULT_WORDS)) {
    if (SAME.has(at)) continue;
    assert.notDeepEqual(said(ITALIAN, at), said(ENGLISH, at), `${at} is still English`);
  }
});

test('counted words agree with 1 and with 3', () => {
  assert.equal(ITALIAN.labels.count(1), '1 etichetta');
  assert.equal(ITALIAN.labels.count(3), '3 etichette');
  assert.equal(ITALIAN.labels.pasted(1), '1 etichetta aggiunta');
  assert.equal(ITALIAN.labels.pasted(3), '3 etichette aggiunte');
  assert.equal(ITALIAN.attachments.count(1), '1 allegato');
  assert.equal(ITALIAN.attachments.count(3), '3 allegati');
  assert.equal(ENGLISH.labels.count(1), '1 label');
  assert.equal(ENGLISH.attachments.count(3), '3 attachments');
});

test('the calendar’s arrows follow lib/calendar-words.ts', () => {
  assert.equal(ITALIAN.date.previous('month'), 'Mese precedente');
  assert.equal(ITALIAN.date.next('month'), 'Mese successivo');
  assert.equal(ITALIAN.date.previous('year'), 'Anno precedente');
  assert.equal(ITALIAN.date.next('year'), 'Anno successivo');
  assert.equal(ITALIAN.date.previous('decade'), 'Pagina precedente');
  assert.equal(ITALIAN.date.next('decade'), 'Pagina successiva');
});

test('a language gives the same object every time', () => {
  assert.equal(qamilleWords(true), qamilleWords(true));
  assert.equal(qamilleWords(false), qamilleWords(false));
  assert.notEqual(qamilleWords(true), qamilleWords(false));
});

test('a key added by Sigrid fails here', () => {
  const added = { ...DEFAULT_WORDS, common: { ...DEFAULT_WORDS.common } };
  added.common.reopen = 'Reopen';
  added.fresh = { word: 'Word' };
  const found = problems(added, ITALIAN);
  assert.ok(found.includes('common.reopen is missing'), found.join('; '));
  assert.ok(found.includes('fresh.word is missing'), found.join('; '));
  const removed = { ...ITALIAN, search: {} };
  assert.ok(problems(DEFAULT_WORDS, removed).includes('search.clear is missing'));
  const wrong = { ...ITALIAN, search: { clear: () => 'x' } };
  assert.match(problems(DEFAULT_WORDS, wrong).join(';'), /search\.clear is a function/);
});
