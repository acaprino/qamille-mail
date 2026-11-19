// Browser-only stand-in for the Rust ContactService, over a small sample book.
// It mirrors the observable rules (required book, read-only mail cards, three answers
// to a duplicate) but never talks to Google or reads local user data.
import type {
  Account,
  BookAction,
  BookView,
  ContactFields,
  ContactView,
  DuplicateView,
  Email,
  MergeOutcome,
  MergePreview,
} from './types';
import { emptyContact } from './types';

type Source = {
  key: string;
  book: string;
  fields: ContactFields;
  modified: number;
  sync: 'Synced' | 'Pending';
};
type Link = { keys: string[] };
type Journal = { id: number; before: Source[]; created: string[]; linked: string[] };

const card = (name: string, email: string, org = ''): ContactFields => ({
  ...emptyContact(),
  name,
  org,
  emails: [{ label: 'work', value: email }],
});
let sources: Source[] = [
  { key: 'c1', book: 'local', fields: card('Ada Lovelace', 'ada@example.test', 'Analytical'), modified: 100, sync: 'Synced' },
  { key: 'c2', book: 'google:1', fields: card('Ada L.', 'ada@example.test'), modified: 90, sync: 'Synced' },
  { key: 'c3', book: 'local', fields: card('Mario Rossi', 'mario@acme.test', 'Acme'), modified: 80, sync: 'Synced' },
  { key: 'c4', book: 'google:2', fields: card('Mario Rossi', 'm.rossi@acme.test', 'Acme'), modified: 70, sync: 'Synced' },
  { key: 'c5', book: 'local', fields: card('Bea Verdi', 'bea@example.test'), modified: 60, sync: 'Synced' },
];
let links: Link[] = [];
let rejected: [string, string][] = [];
let hidden: string[] = [];
let journals: Journal[] = [];
let nextKey = 100;
let lastBook: string | null = null;
let syncOff: string[] = [];
let priority: string[] = [];
let roles: string[] = [];
let readAllowed = false;

const ROLE = ['info', 'sales', 'support', 'noreply', 'no-reply', 'admin'];
const lower = (v: string) => v.trim().toLowerCase();
const nameKey = (v: string) =>
  v
    .normalize('NFD')
    .replace(/[̀-ͯ'’`]/g, '')
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean)
    .sort()
    .join(' ');
const isRole = (email: string) => [...ROLE, ...roles].includes(lower(email).split('@')[0]);
const writable = (book: string) => book === 'local' || /^google:\d+$/.test(book);
const kindOf = (book: string): BookView['kind'] =>
  book.startsWith('google-other:') ? 'GoogleOther' : book.startsWith('google:') ? 'Google' : book.startsWith('mail:') ? 'Mail' : 'Local';
const accountOf = (book: string) => Number(book.split(':')[1]);

function mailSources(emails: Email[]): Source[] {
  const seen = new Map<string, Source>();
  for (const e of emails) {
    const add = (name: string, address: string) => {
      const addr = lower(address);
      if (!addr) return;
      const key = `mail:${e.account_id}:${addr}`;
      if (!seen.has(key))
        seen.set(key, { key, book: `mail:${e.account_id}`, fields: card(name, addr), modified: 0, sync: 'Synced' });
    };
    add(e.from_name, e.from_addr);
    for (const r of [...e.to, ...e.cc]) {
      const m = /^(.*?)\s*<([^>]+)>$/.exec(r);
      add(m ? m[1].replace(/^"|"$/g, '') : '', m ? m[2] : r);
    }
  }
  return [...seen.values()];
}

function groups(all: Source[]): string[][] {
  const parent = new Map(all.map((s) => [s.key, s.key]));
  const find = (k: string): string => {
    let r = k;
    while (parent.get(r) !== r) r = parent.get(r)!;
    parent.set(k, r);
    return r;
  };
  const union = (a: string, b: string) => {
    const [ra, rb] = [find(a), find(b)];
    if (ra !== rb && !rejected.some(([x, y]) => (find(x) === ra && find(y) === rb) || (find(x) === rb && find(y) === ra))) parent.set(rb, ra);
  };
  const byEmail = new Map<string, string>();
  for (const s of all)
    for (const e of s.fields.emails) {
      const k = lower(e.value);
      if (!k || isRole(k)) continue;
      if (byEmail.has(k)) union(byEmail.get(k)!, s.key);
      else byEmail.set(k, s.key);
    }
  const out = new Map<string, string[]>();
  for (const s of all) out.set(find(s.key), [...(out.get(find(s.key)) || []), s.key]);
  return [...out.values()];
}

function rank(book: string) {
  const p = priority.indexOf(book);
  return kindOf(book) === 'Mail' ? 1e9 : p < 0 ? priority.length : p;
}
function mergeFields(members: Source[]): ContactFields {
  const ordered = [...members].sort((a, b) => rank(a.book) - rank(b.book) || b.modified - a.modified || a.key.localeCompare(b.key));
  const first = (pick: (f: ContactFields) => string) => ordered.map((s) => pick(s.fields)).find((v) => v.trim()) || '';
  const unite = (pick: (f: ContactFields) => ContactFields['emails'], key: (value: string) => string) => {
    const seen = new Set<string>();
    return ordered.flatMap((s) => pick(s.fields)).filter((i) => {
      const k = key(i.value);
      return k && !seen.has(k) && !!seen.add(k);
    });
  };
  return {
    name: first((f) => f.name),
    org: first((f) => f.org),
    title: first((f) => f.title),
    birthday: first((f) => f.birthday),
    emails: unite((f) => f.emails, lower),
    phones: unite((f) => f.phones, (v) => v.replace(/\D+/g, '').slice(-9)),
    addresses: unite((f) => f.addresses, (v) => v.trim().replace(/\s+/g, ' ').toLowerCase()),
    notes: [...new Set(ordered.map((s) => s.fields.notes.trim()).filter(Boolean))].join('\n\n'),
  };
}

function label(book: string, accounts: Account[]) {
  const email = accounts.find((a) => a.id === accountOf(book))?.email ?? '?';
  return { local: 'Locale', Google: `Google · ${email}`, GoogleOther: `Google · altri · ${email}`, Mail: `Posta · ${email}`, Local: 'Locale' }[kindOf(book)];
}

function unified(emails: Email[], accounts: Account[]): ContactView[] {
  const live = (book: string) => !book.includes(':') || accounts.some((a) => a.id === accountOf(book));
  const all = [...sources.filter((s) => live(s.book)), ...mailSources(emails).filter((s) => live(s.book))];
  const byKey = new Map(all.map((s) => [s.key, s]));
  const parent = new Map(all.map((s) => [s.key, s.key]));
  const find = (k: string): string => {
    while (parent.get(k) !== k) k = parent.get(k)!;
    return k;
  };
  for (const g of [...groups(all), ...links.map((l) => l.keys.filter((k) => byKey.has(k)))])
    for (const k of g.slice(1)) parent.set(find(k), find(g[0]));
  const comps = new Map<string, Source[]>();
  for (const s of all) comps.set(find(s.key), [...(comps.get(find(s.key)) || []), s]);
  return [...comps.values()]
    .map((members) => {
      members.sort((a, b) => rank(a.book) - rank(b.book) || a.key.localeCompare(b.key));
      return {
        id: members.map((m) => m.key).sort()[0],
        fields: mergeFields(members),
        hidden: members.every((m) => hidden.includes(m.key)),
        origins: members.map((m) => ({
          key: m.key,
          book: m.book,
          label: label(m.book, accounts) as string,
          writable: writable(m.book),
          sync: m.sync,
          modified: m.modified,
        })),
      };
    })
    .sort((a, b) => a.fields.name.toLowerCase().localeCompare(b.fields.name.toLowerCase()) || a.id.localeCompare(b.id));
}

function books(accounts: Account[]): BookView[] {
  const view = (id: string, kind: BookView['kind'], canWrite: boolean): BookView => ({
    id,
    label: label(id, accounts) as string,
    kind,
    writable: canWrite,
    sync_enabled: !syncOff.includes(id),
    last_sync: null,
    write_granted: canWrite,
  });
  return [
    view('local', 'Local', true),
    ...accounts.flatMap((a) => [
      view(`google:${a.id}`, 'Google', true),
      view(`google-other:${a.id}`, 'GoogleOther', false),
      view(`mail:${a.id}`, 'Mail', false),
    ]),
  ];
}

const matches = (f: ContactFields, text: string) => {
  const needle = text.trim().toLowerCase();
  if (!needle) return true;
  return [f.name, f.org, f.title, f.notes, ...f.emails.map((e) => e.value), ...f.phones.map((p) => p.value)].some((v) => v.toLowerCase().includes(needle));
};
const find = (views: ContactView[], id: string) => {
  const found = views.find((c) => c.id === id || c.origins.some((o) => o.key === id));
  if (!found) throw new Error('Contact not found');
  return found;
};
const strings = (value: unknown) => (Array.isArray(value) ? value.map(String) : []);

function plan(views: ContactView[], keys: string[], keep: string[], accounts: Account[]) {
  if (keys.length < 2) throw new Error('Select at least two cards');
  if (!keep.length) throw new Error('Choose at least one address book');
  if (keep.some((b) => !writable(b))) throw new Error('This address book cannot receive contacts');
  const members = keys.map((k) => sources.find((s) => s.key === k) ?? mailSources([]).find((s) => s.key === k));
  if (members.some((m) => !m)) throw new Error('Contact not found');
  const cards = members as Source[];
  const involved = [...new Set([...keep, ...cards.filter((c) => writable(c.book)).map((c) => c.book)])].sort();
  const actions = involved.flatMap((book): BookAction[] => {
    const copies = cards.filter((c) => c.book === book && writable(c.book));
    const base = { book, label: label(book, accounts) as string };
    if (keep.includes(book))
      return copies.length
        ? [{ ...base, key: copies[0].key, action: 'Update' }, ...copies.slice(1).map((c): BookAction => ({ ...base, key: c.key, action: 'Delete' }))]
        : [{ ...base, key: null, action: 'Create' }];
    return copies.map((c): BookAction => ({ ...base, key: c.key, action: 'Delete' }));
  });
  void views;
  return { cards, actions };
}

export function previewContacts(
  command: string,
  args: Record<string, unknown>,
  accounts: Account[],
  emails: Email[],
): { handled: true; value: unknown } | { handled: false } {
  if (!command.startsWith('contacts_')) return { handled: false };
  const views = () => unified(emails, accounts);
  const done = (value: unknown = undefined) => ({ handled: true as const, value: structuredClone(value) });
  const now = () => Math.floor(Date.now() / 1000);
  switch (command) {
    case 'contacts_list': {
      const q = (args.query || {}) as { text?: string; book?: string | null; include_hidden?: boolean; offset?: number; limit?: number };
      const rows = views().filter((c) => (q.include_hidden || !c.hidden) && (!q.book || c.origins.some((o) => o.book === q.book)) && matches(c.fields, q.text || ''));
      const limit = q.limit || 60;
      const offset = q.offset || 0;
      return done({ items: rows.slice(offset, offset + limit), total: rows.length, next_offset: offset + limit < rows.length ? offset + limit : null });
    }
    case 'contacts_get':
      return done(find(views(), String(args.id)));
    case 'contacts_books':
      return done(books(accounts));
    case 'contacts_similar': {
      const fields = args.fields as ContactFields;
      const mine = fields.emails.map((e) => lower(e.value)).filter(Boolean);
      const name = nameKey(fields.name);
      return done(views().filter((c) => (name && nameKey(c.fields.name) === name) || c.fields.emails.some((e) => mine.includes(lower(e.value)))));
    }
    case 'contacts_create': {
      const book = String(args.book || '').trim();
      if (!book) throw new Error('Choose an address book');
      if (!writable(book)) throw new Error('This address book cannot receive new contacts');
      const fields = args.fields as ContactFields;
      if (!fields.name.trim() && !fields.emails.some((e) => e.value.trim()) && !fields.phones.some((p) => p.value.trim()))
        throw new Error('Add a name, an email address or a phone number');
      const key = `c${nextKey++}`;
      sources.push({ key, book, fields: structuredClone(fields), modified: now(), sync: book === 'local' ? 'Synced' : 'Pending' });
      lastBook = book;
      return done(find(views(), key));
    }
    case 'contacts_prefill':
      return done(find(views(), String(args.id)).fields);
    case 'contacts_update_targets': {
      const writableBooks = find(views(), String(args.id)).origins.filter((o) => o.writable).map((o) => o.book);
      return done(books(accounts).filter((b) => writableBooks.includes(b.id)));
    }
    case 'contacts_update': {
      const view = find(views(), String(args.id));
      const targets = view.origins.filter((o) => o.writable).map((o) => o.key);
      if (!targets.length) throw new Error('This contact is only seen in mail. Add it to an address book first');
      for (const s of sources.filter((x) => targets.includes(x.key))) {
        s.fields = structuredClone(args.fields as ContactFields);
        s.modified = now();
        if (s.book !== 'local') s.sync = 'Pending';
      }
      return done(find(views(), view.id));
    }
    case 'contacts_delete': {
      const view = find(views(), String(args.id));
      const chosen = strings(args.books);
      if (chosen.some((b) => !writable(b))) throw new Error('Cards seen in mail cannot be deleted; hide them instead');
      const drop = view.origins.filter((o) => chosen.includes(o.book)).map((o) => o.key);
      if (!drop.length) throw new Error('The contact is not in that address book');
      sources = sources.filter((s) => !drop.includes(s.key));
      links = links.map((l) => ({ keys: l.keys.filter((k) => !drop.includes(k)) })).filter((l) => l.keys.length >= 2);
      return done();
    }
    case 'contacts_hide': {
      const view = find(views(), String(args.id));
      const keys = view.origins.map((o) => o.key);
      hidden = hidden.filter((h) => !keys.includes(h));
      if (args.hidden) hidden.push(...keys);
      return done();
    }
    case 'contacts_suggest': {
      const needle = lower(String(args.text || ''));
      if (!needle) return done([]);
      const out: { name: string; address: string; books: string[] }[] = [];
      for (const c of views().filter((v) => !v.hidden))
        for (const e of c.fields.emails)
          if (`${c.fields.name} ${e.value}`.toLowerCase().includes(needle) && !out.some((o) => lower(o.address) === lower(e.value)))
            out.push({ name: c.fields.name, address: e.value, books: c.origins.map((o) => o.book) });
      return done(out.slice(0, Number(args.limit) || 8));
    }
    case 'contacts_duplicates': {
      const own = sources.filter((s) => writable(s.book));
      const out: DuplicateView[] = [];
      const covered = (keys: string[]) => links.some((l) => keys.every((k) => l.keys.includes(k)));
      for (const g of groups(own)) {
        if (g.length < 2 || covered(g)) continue;
        out.push({ id: g.join('|'), level: 'Certain', members: g.map((k) => member(k, accounts)) });
      }
      const byName = new Map<string, Source[]>();
      for (const s of own) if (nameKey(s.fields.name)) byName.set(nameKey(s.fields.name), [...(byName.get(nameKey(s.fields.name)) || []), s]);
      for (const same of byName.values())
        for (let i = 0; i < same.length; i++)
          for (let j = i + 1; j < same.length; j++) {
            const [a, b] = [same[i], same[j]];
            const pair = [a.key, b.key].sort() as [string, string];
            const sameOrg = a.fields.org && nameKey(a.fields.org) === nameKey(b.fields.org);
            const certain = groups(own).some((g) => g.includes(a.key) && g.includes(b.key));
            if (certain || covered(pair) || rejected.some(([x, y]) => x === pair[0] && y === pair[1])) continue;
            if (sameOrg || args.includePossible) out.push({ id: pair.join('|'), level: sameOrg ? 'Probable' : 'Possible', members: pair.map((k) => member(k, accounts)) });
          }
      const order = { Certain: 0, Probable: 1, Possible: 2 };
      return done(out.sort((a, b) => order[a.level] - order[b.level] || a.id.localeCompare(b.id)));
    }
    case 'contacts_same_person': {
      const keys = strings(args.keys);
      if (keys.length < 2) throw new Error('Select at least two cards');
      links = [...links.filter((l) => !l.keys.some((k) => keys.includes(k))), { keys: [...new Set([...keys, ...links.filter((l) => l.keys.some((k) => keys.includes(k))).flatMap((l) => l.keys)])] }];
      rejected = rejected.filter(([a, b]) => !(keys.includes(a) && keys.includes(b)));
      return done();
    }
    case 'contacts_unlink': {
      const keys = strings(args.keys);
      links = links.map((l) => ({ keys: l.keys.filter((k) => !keys.includes(k)) })).filter((l) => l.keys.length >= 2);
      return done();
    }
    case 'contacts_different_people': {
      const keys = strings(args.keys);
      if (keys.length < 2) throw new Error('Select at least two cards');
      links = links.map((l) => ({ keys: l.keys.filter((k) => !keys.includes(k)) })).filter((l) => l.keys.length >= 2);
      for (let i = 0; i < keys.length; i++) for (let j = i + 1; j < keys.length; j++) rejected.push([keys[i], keys[j]].sort() as [string, string]);
      return done();
    }
    case 'contacts_merge_preview': {
      const { cards, actions } = plan(views(), strings(args.keys), strings(args.keepBooks), accounts);
      const result: MergePreview = { result: mergeFields(cards), actions };
      return done(result);
    }
    case 'contacts_merge_apply': {
      const keys = strings(args.keys);
      const { cards, actions } = plan(views(), keys, strings(args.keepBooks), accounts);
      const fields = structuredClone(args.fields as ContactFields);
      const journal: Journal = { id: nextKey++, before: structuredClone(cards.filter((c) => writable(c.book))), created: [], linked: [] };
      const kept: string[] = [];
      for (const a of actions) {
        if (a.action === 'Create') {
          const key = `c${nextKey++}`;
          sources.push({ key, book: a.book, fields: structuredClone(fields), modified: now(), sync: a.book === 'local' ? 'Synced' : 'Pending' });
          journal.created.push(key);
          kept.push(key);
        } else if (a.action === 'Update') {
          const s = sources.find((x) => x.key === a.key)!;
          s.fields = structuredClone(fields);
          s.modified = now();
          kept.push(s.key);
        } else if (a.action === 'Delete') sources = sources.filter((x) => x.key !== a.key);
      }
      journal.linked = kept;
      if (kept.length > 1) links.push({ keys: kept });
      rejected = rejected.filter(([a, b]) => !(keys.includes(a) && keys.includes(b)));
      journals.push(journal);
      const outcome: MergeOutcome = { journal_id: journal.id, per_book: [...new Set(actions.map((a) => a.book))].map((book) => ({ book, ok: true, error: null })), recreated: [] };
      return done(outcome);
    }
    case 'contacts_merge_undo': {
      const journal = journals.find((j) => j.id === Number(args.journalId));
      if (!journal) throw new Error('Merge history not found');
      journals = journals.filter((j) => j !== journal);
      sources = sources.filter((s) => !journal.created.includes(s.key));
      for (const before of journal.before) {
        const at = sources.findIndex((s) => s.key === before.key);
        if (at >= 0) sources[at] = before;
        else sources.push(before);
      }
      links = links.map((l) => ({ keys: l.keys.filter((k) => !journal.linked.includes(k)) })).filter((l) => l.keys.length >= 2);
      return done({ journal_id: journal.id, per_book: [], recreated: [] });
    }
    case 'contacts_import_vcf': {
      const count = (String(args.text).match(/BEGIN:VCARD/gi) || []).length;
      return done({ added: 0, skipped: count });
    }
    case 'contacts_export_vcf':
      return done('');
    case 'contacts_sync': {
      const missing = !readAllowed && accounts.length > 0;
      return done({
        books: missing
          ? [{ book: `google:${accounts[0].id}`, added: 0, updated: 0, removed: 0, full: false, skipped: false, error: 'Allow access to Google Contacts for this account first', consent_required: true }]
          : [],
        writes: [],
      });
    }
    case 'contacts_sync_book':
      return done({ book: String(args.book), added: 0, updated: 0, removed: 0, full: false, skipped: false, error: null, consent_required: false });
    case 'contacts_set_sync': {
      const book = String(args.book);
      syncOff = syncOff.filter((b) => b !== book);
      if (!args.enabled) syncOff.push(book);
      return done();
    }
    case 'contacts_authorize':
      if (!args.write) readAllowed = true;
      return done();
    case 'contacts_resolve_conflict':
    case 'contacts_flush':
      return done(command === 'contacts_flush' ? [] : undefined);
    case 'contacts_promote_other':
      throw new Error('Open the desktop app to copy Google other contacts.');
    case 'contacts_settings':
      return done({ priority, roles, last_book: lastBook, sync_off: syncOff, write_granted: [] });
    case 'contacts_save_settings':
      priority = strings(args.priority);
      roles = strings(args.roles);
      return done();
    default:
      throw new Error('This feature needs the desktop app. The browser shows sample mail only.');
  }
}

function member(key: string, accounts: Account[]) {
  const s = sources.find((x) => x.key === key)!;
  return { key, book: s.book, label: label(s.book, accounts) as string, writable: writable(s.book), fields: structuredClone(s.fields) };
}
