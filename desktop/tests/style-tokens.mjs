// Every visual value comes from desktop/src/ds/tokens/theme.css (ADR-0008, ADR-0009, ADR-0011).
// Fails on literal colors, z-index layers, radii and pixel font sizes anywhere else in src/.
// Run: node tests/style-tokens.mjs
import fs from 'node:fs';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '../src');
const THEME = path.join(root, 'ds/tokens/theme.css');

// Literal values that cannot be tokens, with the reason.
const ALLOWED = [
  { file: 'styles.css', text: '#000', why: 'mask-image gradient: only the alpha channel is read' },
  { file: 'lib/message-html.ts', text: '#fff', why: 'received email renders in a sandboxed frame that cannot read app tokens' },
  { file: 'lib/message-html.ts', text: '#1f2328', why: 'received email renders in a sandboxed frame that cannot read app tokens' },
  { file: 'lib/message-html.ts', text: '#2563eb', why: 'received email renders in a sandboxed frame that cannot read app tokens' },
  // The calendar identity palette: eight colors chosen per calendar, not theme colors (the 9
  // literals of the baseline that ADR-0009 recorded before this change).
  ...['#416adb', '#d9534f', '#2f9e6e', '#b7791f', '#8e5bd1', '#0e8fa8', '#d6568f', '#6b7280'].map((text) => ({
    file: 'components/Calendar',
    text,
    why: 'calendar identity palette, one color per calendar',
  })),
  { file: 'components/Calendar', text: 'calc(var(--radius-md) - 2px', why: 'inner radius derived from a token' },
  // Google's «G» on the «Sign in with Google» buttons: Google's branding fixes its four colours.
  ...['#EA4335', '#4285F4', '#FBBC05', '#34A853'].map((text) => ({
    file: 'ds/primitives/GoogleMark',
    text,
    why: 'Google sign-in mark, colours fixed by Google branding',
  })),
];

const RULES = [
  { name: 'literal color', re: /(?<![&\w])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})\b/g, hint: 'use a color token' },
  { name: 'literal color function', re: /\b(?:rgba?|hsla?)\((?!\$\{)/g, hint: 'use a color token' },
  { name: 'global z-index', re: /z-index:\s*(?:[6-9]|\d{2,})\b/g, hint: 'use --z-* (only 1–5 for stacking inside a component)' },
  { name: 'literal radius', re: /border-radius:[^;]*?\b[1-9]\d*px/g, hint: 'use --radius-*' },
  { name: 'pixel font size', re: /font-size:\s*\d+(?:\.\d+)?px/g, hint: 'use --text-*' },
];

function files(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    // ds/unlumen is checked too: the vendored components read the tokens like the rest (ADR-0011 v3 INV-008).
    if (entry.isDirectory()) return files(full);
    return /\.(css|tsx?)$/.test(entry.name) && full !== THEME ? [full] : [];
  });
}

// Comments may mention values; blank them but keep line numbers.
const stripComments = (text) =>
  text
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, ' '))
    .replace(/<!--[\s\S]*?-->/g, (m) => m.replace(/[^\n]/g, ' '))
    .replace(/^\s*\/\/.*$/gm, (m) => ' '.repeat(m.length));

const problems = [];
for (const file of files(root)) {
  const rel = path.relative(root, file).replaceAll('\\', '/');
  const lines = stripComments(fs.readFileSync(file, 'utf8')).split('\n');
  lines.forEach((line, i) => {
    for (const rule of RULES) {
      for (const match of line.matchAll(rule.re)) {
        const value = match[0];
        if (ALLOWED.some((a) => (a.file === rel || rel.startsWith(`${a.file}.`)) && value.toLowerCase().includes(a.text.toLowerCase()))) continue;
        problems.push(`${rel}:${i + 1}  ${rule.name} "${value}": ${rule.hint}`);
      }
    }
  });
}

if (problems.length) {
  console.error(problems.join('\n'));
  console.error(`\nFAIL ${problems.length} literal style values outside theme.css`);
  process.exit(1);
}
console.log('PASS every color, layer, radius and font size comes from theme.css');
