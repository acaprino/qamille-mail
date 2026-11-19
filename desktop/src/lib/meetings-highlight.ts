// How a search result shows its fragment: the words that were searched, marked. Pure text in,
// pieces out, so the page never builds HTML from what the user typed or what a meeting says (no
// {@html}); the component draws each piece, a <mark> for the ones that hit.

export interface MarkedPart {
  text: string;
  hit: boolean;
}

const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** The words of a query, as the backend reads them (`search_details`): split on whitespace. */
export function searchTerms(query: string): string[] {
  return query.split(/\s+/).filter(Boolean);
}

/** One case-insensitive pattern for all the terms, the longest first so `prototipo` beats `pro`. */
function pattern(query: string): RegExp | null {
  const terms = searchTerms(query).sort((a, b) => b.length - a.length);
  return terms.length ? new RegExp(terms.map(escapeRegExp).join('|'), 'giu') : null;
}

/** `text` in pieces, the ones that match a word of the query flagged. The pieces add up to `text`. */
export function markedParts(text: string, query: string): MarkedPart[] {
  if (!text) return [];
  const find = pattern(query);
  if (!find) return [{ text, hit: false }];
  const parts: MarkedPart[] = [];
  let from = 0;
  for (const match of text.matchAll(find)) {
    const at = match.index ?? 0;
    if (!match[0]) continue;
    if (at > from) parts.push({ text: text.slice(from, at), hit: false });
    parts.push({ text: match[0], hit: true });
    from = at + match[0].length;
  }
  if (from < text.length) parts.push({ text: text.slice(from), hit: false });
  return parts;
}

/** How far to look for a space to cut at, so a word is not cut in half. */
const SNAP = 14;

/**
 * About `width` characters of `text` around the first match (from the start when nothing matches),
 * on one line, with … where it was cut. Long summaries and long segments become one readable line.
 */
export function fragmentAround(text: string, query: string, width = 140): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  if (flat.length <= width) return flat;
  const find = pattern(query);
  const first = find ? flat.search(find) : -1;
  let start = first > 0 ? Math.max(0, first - Math.floor(width / 3)) : 0;
  if (start > 0) {
    const space = flat.indexOf(' ', start);
    if (space !== -1 && space - start <= SNAP && space < first) start = space + 1;
  }
  let end = Math.min(flat.length, start + width);
  if (end < flat.length) {
    const space = flat.lastIndexOf(' ', end);
    if (space > start && end - space <= SNAP && space > first) end = space;
  }
  return `${start > 0 ? '…' : ''}${flat.slice(start, end).trim()}${end < flat.length ? '…' : ''}`;
}
