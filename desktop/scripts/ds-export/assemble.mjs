// Builds the files of the design system from the captured cards, the tokens and the module's sources. Pure:
// it returns a Map of relative path -> string or Buffer, and writes nothing, so what it returns is what is tested.
import fs from 'node:fs';
import path from 'node:path';
import { desktop, templates } from './config.mjs';
import { ABOVE_BREAKPOINTS, INFO, PADS, PROP_NOTES, WIDE, WRAP } from './components.mjs';
import { declaredTypes, readProps, resolveType, scriptOf } from './props.mjs';
import { readText } from './module-source.mjs';

const INK_FALLBACK = 'currentColor';

// Geometry of a preview card from the parts captured in it.
function cardGeometry(comp, parts) {
  const multi = parts.length > 1;
  const pad = PADS[comp] ?? 0;
  const wrapW = WRAP[comp] ?? 640;
  const inner = wrapW - 32;
  let wrapHeight = 32;
  if (multi) {
    let rowW = 0;
    let rowH = 0;
    let total = 0;
    for (const part of parts) {
      if (rowW && rowW + 12 + part.w > inner) { total += rowH + 12; rowW = 0; rowH = 0; }
      rowW += (rowW ? 12 : 0) + part.w;
      rowH = Math.max(rowH, part.h);
    }
    wrapHeight += total + rowH;
  }
  const width = multi ? wrapW : Math.max(...parts.map((p) => p.w)) + pad * 2;
  const height = multi ? wrapHeight : Math.max(...parts.map((p) => p.h)) + pad * 2;
  const cardWidth = WIDE.has(comp) ? 1440 : (ABOVE_BREAKPOINTS.has(comp) ? 1201 : width);
  return { multi, pad, width, height, cardWidth, left: Math.floor((cardWidth - width) / 2) };
}

function previewHtml(f) {
  const g = cardGeometry(f.comp, f.parts);
  // data-case only names a case of the harness page: it means nothing in a card.
  const safe = f.html.replace(/<script[\s\S]*?<\/script>/g, '').replace(/ data-case="[^"]*"/g, '');
  const scroll = safe.includes('data-st=') ? '\n<script>document.querySelectorAll("[data-st]").forEach(function(e){var s=e.getAttribute("data-st").split(",");e.scrollTop=+s[0];e.scrollLeft=+s[1]});</script>' : '';
  const subtitle = f.source === 'module' ? 'Anteprima montata dal componente del modulo' : 'Anteprima statica dal DOM reale';
  return `<!-- @dsCard group="${f.group}" height=${Math.min(g.height, 4000)} width=${g.cardWidth} subtitle="${subtitle}" -->
<!doctype html>
<html lang="it">
<head><meta charset="utf-8"><title>${f.comp}</title></head>
<body style="margin:0">
<div class="${g.multi ? 'ds-wrap' : 'ds-page'}" style="width:${g.width}px;margin-left:${g.left}px;${g.multi ? '' : `height:${g.height}px;overflow:hidden;padding:${g.pad}px;box-sizing:border-box`}">${safe}</div>${scroll}
</body>
</html>
`;
}

// The d.ts declaration of a component's props, and the imports it needs.
function declaration(comp, source, imports) {
  const props = readProps(source.abs);
  if (props.kind === 'none') return '';
  const resolved = resolveType(source.abs, props.type);
  if (!resolved) return '';
  const text = resolved.kind === 'interface' ? resolved.body : resolved.text;
  const lines = declaredTypes(source.abs, text).map((t) => `export type ${t.name} = ${t.text};`);
  if (/\bSnippet\b/.test(text)) imports.add("import type { Snippet } from 'svelte';");
  for (const m of text.matchAll(/\bHTML\w*Attributes\b/g)) imports.add(m[0]);
  // A type the props take from a package (Ark's own, for the translations) is imported the way the component imports it.
  const own = new Map();
  for (const m of scriptOf(source.abs).matchAll(/import\s+(?:type\s+)?\{([^}]*)\}\s+from\s+'([^'.][^']*)'/g)) {
    for (const n of m[1].split(',').map((x) => x.trim()).filter(Boolean)) own.set(n, m[2]);
  }
  for (const id of new Set([...`${lines.join('\n')}\n${text}`.matchAll(/\b[A-Z]\w*\b/g)].map((x) => x[0]))) {
    if (own.has(id) && id !== 'Snippet' && !/^HTML\w*Attributes$/.test(id)) imports.add(`import type { ${id} } from '${own.get(id)}';`);
  }
  lines.push(resolved.kind === 'interface' ? `export interface ${comp}Props ${resolved.body}` : `export type ${comp}Props = ${resolved.text};`);
  return `${lines.join('\n')}\n`;
}

function componentReadme(f, source) {
  const info = INFO[f.comp];
  const width = f.parts.length === 1 ? f.parts[0].w : Math.max(...f.parts.map((p) => p.w));
  const lines = [info[0], '', info[1].replaceAll('{w}', String(width)), ''];
  const where = source ? source.file : info[2];
  lines.push(
    f.source === 'module'
      ? `**Sorgente:** \`${where}\`. Componente Svelte del modulo: l’anteprima è montata dal componente stesso, con le classi di \`bundle.css\`.`
      : `**Sorgente:** \`${where}\`. ${source ? 'Componente Svelte: l’anteprima è una resa statica del suo DOM, con le classi di `bundle.css`.' : 'Classi del sistema: l’anteprima è una resa statica del DOM dell’app, con le classi di `bundle.css`.'}`,
    '',
  );
  if (source) {
    const p = readProps(source.abs);
    const notes = PROP_NOTES[f.comp] ?? {};
    if (p.kind !== 'none' && p.names.length) {
      lines.push('**Cosa fornire:**', '');
      for (const n of p.names) {
        const d = p.defaults[n];
        const dflt = d === undefined ? '' : d.startsWith('$bindable') ? ' (si lega con `bind:`)' : ` (predefinito \`${d.replace(/\s+/g, ' ').slice(0, 40)}\`)`;
        lines.push(`- \`${n}\`${dflt}${notes[n] ? `: ${notes[n]}` : ''}`);
      }
      lines.push('');
    } else lines.push('**Cosa fornire:** nessuna prop. Legge lo stato condiviso dell’app.', '');
  }
  return `${lines.join('\n').trim()}\n`;
}

/** Everything the export writes, as a Map of path (relative to the output folder) -> contents. */
export function assemble({ tokens, frags, bundleCss, icons, bolt, fonts, digest, tokenCount, components, blobs, at, partial }) {
  const out = new Map();
  const put = (file, data) => out.set(file, data);
  const colors = Object.fromEntries(tokens.color.tokens.map((t) => [t.name, t.value.light]));
  const INK = colors.text ?? INK_FALLBACK;
  const ACCENT = colors.accent ?? INK_FALLBACK;
  const comps = [...frags].sort((a, b) => (a.comp < b.comp ? -1 : a.comp > b.comp ? 1 : 0));

  const imports = new Set();
  const dts = [];
  for (const f of comps) {
    const info = INFO[f.comp];
    if (!info) throw new Error(`assemble: no description for ${f.comp} (add it to scripts/ds-export/components.mjs)`);
    put(`project/components/${f.comp}/preview.html`, previewHtml(f));
    const source = components[f.comp];
    put(`project/components/${f.comp}/README.md`, componentReadme(f, source));
    if (source) dts.push(declaration(f.comp, source, imports));
  }
  const header = '// Le prop reali dei componenti Svelte di Qamille. Documentazione, non codice da importare: questo sistema non ha un bundle React.\n';
  const types = [...imports].filter((i) => i.startsWith('import')).sort();
  const attrs = [...imports].filter((i) => !i.startsWith('import')).sort();
  const importLines = [...types, ...(attrs.length ? [`import type { ${attrs.join(', ')} } from 'svelte/elements';`] : [])];
  put('project/components/index.d.ts', `${header}${importLines.length ? `${importLines.join('\n')}\n` : ''}\n${dts.filter(Boolean).join('\n')}`);
  if (partial) return out;

  put('project/tokens.json', JSON.stringify(tokens, null, 1));
  put('project/components/bundle.css', bundleCss);
  for (const f of fonts) put(`project/fonts/${f.file}`, fs.readFileSync(f.source));
  put('project/README.md', readText(path.join(desktop, 'src/ds/README.md')));
  put('project/components/Cover/preview.html', readText(path.join(templates, 'cover.html')));

  // Icons and logo, copied from the sources. <img> cannot inherit color, so the ink is written in and named in the README.
  const svg = (d) => `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="${INK}" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="${d}"/></svg>\n`;
  const bolt21 = `<svg xmlns="http://www.w3.org/2000/svg" width="21" height="25" viewBox="0 0 21 25" fill="none"><path d="${bolt}" fill="${ACCENT}"/></svg>\n`;
  for (const [name, d] of icons) put(`upload/Icons/${name}.svg`, svg(d));
  put('upload/Logos/qamille-bolt.svg', bolt21);
  put('upload/Logos/app-icon.png', fs.readFileSync(path.join(desktop, 'src-tauri/icons/icon.png')));
  put('project/assets/Icons/README.md', `Icone del componente \`Icon\` (\`desktop/src/ds/primitives/Icon.svelte\`), con gli stessi tracciati. Griglia 24×24, tratto 1,7. Inchiostro dei file: \`${INK}\`, il \`text\` del tema chiaro; nell’app sono in \`currentColor\`.\n`);
  put('project/assets/Logos/README.md', `Il fulmine di \`desktop/src/components/Sidebar.svelte\` e l’icona dell’app di \`desktop/src-tauri/icons/icon.png\`. Inchiostro del fulmine in questo file: \`${ACCENT}\`, l’\`accent\` del tema chiaro; nell’app è \`on-accent\` su un tassello \`accent\`.\n`);

  // The index. The upload ids and the dates exist only once the files are published: they come in with
  // --blobs and --at, and are absent from the files an ordinary export writes.
  const record = (group, file) => {
    const data = out.get(`upload/${group}/${file}`);
    const r = { name: file };
    const blob = blobs?.[group]?.[file];
    if (blob) r.blob = blob;
    r.size = data.length;
    r.type = file.endsWith('.png') ? 'image/png' : 'image/svg+xml';
    return r;
  };
  const iconFiles = Object.fromEntries(icons.map(([name]) => [`${name}.svg`, record('Icons', `${name}.svg`)]));
  const logoFiles = Object.fromEntries(['qamille-bolt.svg', 'app-icon.png'].map((file) => [file, record('Logos', file)]));
  const created = { v: 1 };
  if (at) created.at = at;
  const change = { by: 'Alfio Caprino' };
  if (at) change.at = at;
  change.via = 'npm run ds:export';
  change.note = `Esportato da desktop/src/ds (${digest}): ${comps.length + 1} componenti, ${tokenCount} variabili di tema, ${icons.length} icone.`;
  put('project/design-system.json', JSON.stringify({
    v: 3,
    layout: 'files',
    createdOnFiles: created,
    title: 'Qamille',
    namespace: 'Qamille',
    libraries: [],
    sections: {},
    groups: ['Logos', 'Icons', 'Licenses'],
    assetGroups: {
      Logos: { name: 'Logos', tile: 'l', order: Object.keys(logoFiles), files: logoFiles },
      Icons: { name: 'Icons', tile: 'xs', order: Object.keys(iconFiles), files: iconFiles },
      Licenses: { name: 'Licenses', tile: 'xs', order: [], files: {} },
    },
    blobs: {},
    docs: { sections: [] },
    lastChange: change,
  }, null, 1));

  const map = {};
  for (const file of [...out.keys()].filter((f) => f.startsWith('project/') && f !== 'project/design-system.json').sort()) map[file] = file;
  put('files.json', JSON.stringify(map, null, 1));
  return out;
}
