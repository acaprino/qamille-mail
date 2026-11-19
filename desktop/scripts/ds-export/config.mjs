// Fixed settings of the export. INV-005 (ADR-0011 v2): the same module and the same sample data give the
// same files, so the clock, the locale and the time zone of every capture are written here, once.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
export const src = path.join(desktop, 'src');
export const ds = {
  dir: path.join(src, 'ds'),
  theme: path.join(src, 'ds/tokens/theme.css'),
  system: path.join(src, 'ds/styles/system.css'),
  ark: path.join(src, 'ds/styles/ark.css'),
  primitives: path.join(src, 'ds/primitives'),
  readme: path.join(src, 'ds/README.md'),
};
export const here = path.dirname(fileURLToPath(import.meta.url));
export const templates = path.join(here, 'templates');

// What every capture runs under. The date is a Tuesday afternoon in the sample data's week.
export const FROZEN = {
  time: '2026-11-03T15:00:00+01:00',
  locale: 'it-IT',
  timezone: 'Europe/Rome',
  viewport: { width: 1440, height: 900 },
};
export const PORT = 28180;

// The primitives of the module (desktop/src/ds/primitives), in the order of the guide. Each has a preview in the
// export; tests/ds-export.mjs has its own copy of this list and of what each preview has to show.
export const PRIMITIVES = [
  'Button', 'Field', 'Icon', 'Modal', 'SidePanel', 'Popover', 'Tooltip', 'Select', 'Checkbox', 'Switch', 'Tabs', 'Slider', 'DateField', 'RecipientsInput', 'Toaster',
];

export function parseArgs(argv) {
  const at = (flag) => { const i = argv.indexOf(flag); return i >= 0 ? argv[i + 1] : undefined; };
  const out = path.resolve(at('--out') ?? path.join(desktop, 'ds-export'));
  const only = at('--only')?.split(',').filter(Boolean) ?? null;
  return {
    out,
    only,
    port: at('--port') ? Number(at('--port')) : PORT,
    // Publishing inputs, absent by default so that the files carry no timestamp and no upload id.
    blobs: at('--blobs') ? JSON.parse(fs.readFileSync(path.resolve(at('--blobs')), 'utf8')) : null,
    at: at('--at') ?? null,
  };
}
