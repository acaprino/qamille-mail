// Compares two runs of visual-snapshot.mjs: the same screenshots byte for byte, the same elements,
// boxes and computed styles. Exits non-zero and says what differs.
//
//   node tests/visual-compare.mjs <before-dir> <after-dir>
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';

const [a, b] = process.argv.slice(2).map((dir) => path.resolve(dir));
const read = (dir, name) => JSON.parse(zlib.gunzipSync(fs.readFileSync(path.join(dir, `${name}.json.gz`))));
const states = (dir) => fs.readdirSync(dir).filter((f) => f.endsWith('.json.gz')).map((f) => f.slice(0, -8)).sort();
const left = states(a);
const right = states(b);
let problems = 0;
const say = (text) => {
  problems++;
  if (problems <= 60) console.log(text);
};
for (const name of left.filter((s) => !right.includes(s))) say(`MISSING after: ${name}`);
for (const name of right.filter((s) => !left.includes(s))) say(`NEW after: ${name}`);
let samePixels = 0;
let compared = 0;
for (const name of left.filter((s) => right.includes(s))) {
  compared++;
  if (fs.readFileSync(path.join(a, `${name}.png`)).equals(fs.readFileSync(path.join(b, `${name}.png`)))) samePixels++;
  else say(`PIXELS ${name}`);
  const before = read(a, name);
  const after = read(b, name);
  if (before.length !== after.length) say(`ELEMENTS ${name}: ${before.length} -> ${after.length}`);
  const seen = new Map(after.map((row) => [row.at, row]));
  let shown = 0;
  for (const row of before) {
    const other = seen.get(row.at);
    if (!other) continue;
    const differences = [];
    if (row.tag !== other.tag) differences.push(`tag ${row.tag} -> ${other.tag}`);
    if (row.cls !== other.cls) differences.push(`class "${row.cls}" -> "${other.cls}"`);
    if (row.text !== other.text) differences.push(`text "${row.text}" -> "${other.text}"`);
    if (row.attrs !== other.attrs) differences.push(`attrs "${row.attrs}" -> "${other.attrs}"`);
    if (row.box.join() !== other.box.join()) differences.push(`box ${row.box} -> ${other.box}`);
    for (const key of Object.keys(row.style)) {
      if (row.style[key] !== other.style[key]) differences.push(`${key}: ${row.style[key]} -> ${other.style[key]}`);
    }
    if (differences.length) {
      problems++;
      if (shown++ < 3 && problems <= 60) console.log(`${name} ${row.at} <${row.tag} class="${row.cls}">\n   ${differences.slice(0, 5).join('\n   ')}`);
    }
  }
  if (shown > 3) console.log(`${name}: ${shown} elements differ`);
}
console.log(`compared ${compared} states; screenshots identical: ${samePixels}/${compared}; differences: ${problems}`);
process.exit(problems ? 1 : 0);
