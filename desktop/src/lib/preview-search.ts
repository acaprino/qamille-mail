// Deterministic sample-only search used to exercise the browser UI. Production
// search is evaluated by the shared Rust grammar, never by this adapter.
import type { Email, Store } from './types';
import { configureRegional, dateKey, regionalLocale } from './regional';

type Atom = {
  kind: 'atom';
  value: string;
  literal: boolean;
  op?: string;
  lower?: string;
  upper?: string;
  center?: string;
  duration?: number;
};
type Node =
  | Atom
  | { kind: 'and'; left: Node; right: Node }
  | { kind: 'or'; left: Node; right: Node }
  | { kind: 'not'; child: Node };
type Token = { value: string; literal: boolean; symbol?: boolean };
const normalize = (value: string) => value.normalize('NFC').toLowerCase();
const error = (it: string, en: string): never => {
  throw new Error(regionalLocale().startsWith('it') ? it : en);
};

function tokens(text: string): Token[] {
  const result: Token[] = [];
  for (let i = 0; i < text.length;) {
    if (/\s/.test(text[i])) {
      i++;
      continue;
    }
    if ('()-'.includes(text[i])) {
      result.push({ value: text[i++], literal: false, symbol: true });
      continue;
    }
    let value = '';
    let quoted = false;
    const literal = text[i] === '"';
    while (i < text.length) {
      const char = text[i];
      if (char === '"') {
        quoted = !quoted;
        i++;
        continue;
      }
      if (quoted && char === '\\' && ['"', '\\'].includes(text[i + 1])) {
        value += text[i + 1];
        i += 2;
        continue;
      }
      if (!quoted && (/\s/.test(char) || '()'.includes(char))) break;
      value += char;
      i++;
    }
    if (quoted) error('Chiudi le virgolette.', 'Close the quotation marks.');
    if (!value) error('Inserisci un valore da cercare.', 'Enter a value to search.');
    result.push({ value, literal });
  }
  return result;
}

function canonicalDate(value: string): string {
  let match = /^(\d{4})[-/](\d{2})[-/](\d{2})$/.exec(value);
  let result: string;
  if (match) result = `${match[1]}-${match[2]}-${match[3]}`;
  else {
    match = /^(\d{2})\/(\d{2})\/(\d{4})$/.exec(value);
    if (!match) error('Inserisci una data completa valida.', 'Enter a valid complete date.');
    result = regionalLocale().startsWith('en-US')
      ? `${match![3]}-${match![1]}-${match![2]}`
      : `${match![3]}-${match![2]}-${match![1]}`;
  }
  const date = new Date(`${result}T12:00:00Z`);
  if (!Number.isFinite(date.valueOf()) || date.toISOString().slice(0, 10) !== result)
    error('La data non è valida.', 'The date is invalid.');
  return result;
}

function offset(value: string, days: number): string {
  const date = new Date(`${value}T12:00:00Z`);
  date.setUTCDate(date.getUTCDate() + days);
  return date.toISOString().slice(0, 10);
}

function atom(token: Token, store: Store): Atom {
  const node: Atom = { kind: 'atom', value: token.value, literal: token.literal };
  if (token.literal || !token.value.includes(':')) return node;
  const colon = token.value.indexOf(':');
  const op = token.value.slice(0, colon).toLowerCase();
  const value = token.value.slice(colon + 1);
  if (!value.trim()) error(`Inserisci un valore per ${op}:`, `Enter a value for ${op}:`);
  node.op = op === 'tag' ? 'label' : op === 'folder' ? 'in' : op;
  node.value = value;
  switch (node.op) {
    case 'from':
    case 'to':
    case 'cc':
    case 'subject':
    case 'body':
    case 'filename':
    case 'label':
    case 'type':
      break;
    case 'account':
      if (
        !store.accounts.some((account) =>
          [account.email, account.name].some((field) => normalize(field) === normalize(value)),
        )
      )
        error('Account non riconosciuto.', 'Unknown account.');
      break;
    case 'in':
      if (
        ![
          'inbox',
          'sent',
          'archive',
          'trash',
          'spam',
          'anywhere',
          'snoozed',
          ...store.emails.map((email) => email.folder.toLowerCase()),
        ].includes(value.toLowerCase())
      )
        error('Cartella non riconosciuta.', 'Unknown folder.');
      break;
    case 'category':
      if (
        ![
          'personal',
          'priority',
          'notification',
          'newsletter',
          'invitation',
          'invitation_response',
        ].includes(value.toLowerCase())
      )
        error('Categoria non riconosciuta.', 'Unknown category.');
      break;
    case 'has':
      if (
        !['attachment', 'document', 'spreadsheet', 'presentation', 'reminder'].includes(
          value.toLowerCase(),
        )
      )
        error('Filtro has: non riconosciuto.', 'Unknown has: filter.');
      break;
    case 'is':
      if (
        !['unread', 'read', 'starred', 'pinned', 'unreplied', 'done', 'task', 'snoozed'].includes(
          value.toLowerCase(),
        )
      )
        error('Stato non riconosciuto.', 'Unknown state.');
      break;
    case 'on':
      node.lower = node.upper = canonicalDate(value);
      break;
    case 'after':
      node.lower = canonicalDate(value);
      break;
    case 'before':
      node.upper = offset(canonicalDate(value), -1);
      break;
    case 'between': {
      const values = value.split('..');
      if (values.length !== 2)
        error('Usa between:AAAA-MM-GG..AAAA-MM-GG.', 'Use between:YYYY-MM-DD..YYYY-MM-DD.');
      node.lower = canonicalDate(values[0]);
      node.upper = canonicalDate(values[1]);
      if (node.lower > node.upper)
        error('L’intervallo è invertito.', 'The date range is reversed.');
      break;
    }
    case 'around': {
      let date = value;
      let radius = 6;
      if (value.split('/').length === 4 || (/^\d{4}-/.test(value) && value.includes('/'))) {
        const slash = value.lastIndexOf('/');
        date = value.slice(0, slash);
        const raw = value.slice(slash + 1);
        if (!/^\d+d$/.test(raw))
          error(
            'Il raggio deve essere un numero di giorni, per esempio /6d.',
            'The radius must be a number of days, for example /6d.',
          );
        radius = Number(raw.slice(0, -1));
      }
      if (!Number.isInteger(radius) || radius < 0 || radius > 365)
        error(
          'Il raggio deve essere tra 0 e 365 giorni.',
          'The radius must be between 0 and 365 days.',
        );
      node.center = canonicalDate(date);
      node.lower = offset(node.center, -radius);
      node.upper = offset(node.center, radius);
      break;
    }
    case 'newer_than':
    case 'older_than': {
      const match = /^(\d+)([dwmy])$/.exec(value);
      if (!match)
        error('Durata non valida: usa d, w, m oppure y.', 'Invalid duration: use d, w, m or y.');
      node.duration = Number(match![1]) * ({ d: 1, w: 7, m: 30, y: 365 }[match![2]] || 0) * 86400;
      if (!Number.isSafeInteger(node.duration))
        error('Durata troppo grande.', 'Duration is too large.');
      break;
    }
    default:
      error(`Operatore non riconosciuto: ${op}:`, `Unknown operator: ${op}:`);
  }
  return node;
}

function parse(text: string, store: Store): Node {
  const all = tokens(text);
  let at = 0;
  const is = (value: string) => all[at]?.value === value && !all[at]?.literal;
  function primary(): Node {
    if (is('-')) {
      at++;
      return { kind: 'not', child: primary() };
    }
    if (is('(')) {
      at++;
      const child = or();
      if (!is(')')) error('Chiudi la parentesi.', 'Close the parenthesis.');
      at++;
      return child;
    }
    const token = all[at++];
    if (!token || (!token.literal && [')', 'AND', 'OR'].includes(token.value)))
      error('La ricerca è incompleta.', 'The search is incomplete.');
    return atom(token, store);
  }
  function and(): Node {
    let node = primary();
    while (at < all.length && !is(')') && !is('OR')) {
      if (is('AND')) at++;
      node = { kind: 'and', left: node, right: primary() };
    }
    return node;
  }
  function or(): Node {
    let node = and();
    while (is('OR')) {
      at++;
      node = { kind: 'or', left: node, right: and() };
    }
    return node;
  }
  const tree = or();
  if (at !== all.length) error('Parentesi non valida.', 'Invalid parenthesis.');
  return tree;
}

function address(value: string): string {
  return normalize(/<([^>]+)>/.exec(value)?.[1] || value.trim());
}
function person(value: string, name: string, email: string, own: string[]): boolean {
  const needle = normalize(value);
  const addr = address(email);
  if (needle === 'me') return own.includes(addr);
  if (needle.startsWith('@')) return addr.split('@')[1] === needle.slice(1);
  if (needle.includes('@')) return addr === needle;
  return normalize(name).includes(needle) || (addr.split('@')[0] || '').includes(needle);
}
function fields(email: Email): string[] {
  return [
    email.subject,
    email.from_name,
    email.from_addr,
    ...email.to,
    ...email.cc,
    email.body,
    ...email.labels,
    ...email.attachments.flatMap((file) => [file.name, file.inline_text || '']),
  ];
}

function evaluate(
  tree: Node,
  email: Email,
  store: Store,
  own: string[],
  now: number,
): [boolean, boolean] {
  if (tree.kind === 'not') return [!evaluate(tree.child, email, store, own, now)[0], false];
  if (tree.kind === 'and' || tree.kind === 'or') {
    const a = evaluate(tree.left, email, store, own, now);
    const b = evaluate(tree.right, email, store, own, now);
    return tree.kind === 'and'
      ? [a[0] && b[0], a[1] || b[1]]
      : [a[0] || b[0], (a[0] && a[1]) || (b[0] && b[1])];
  }
  const needle = normalize(tree.value);
  const has = (value: string) => normalize(value).includes(needle);
  const metadata = email as Email & { starred?: boolean; replied?: boolean };
  let match = false;
  switch (tree.op) {
    case undefined:
      match = fields(email).some(has);
      break;
    case 'from':
      match = person(tree.value, email.from_name, email.from_addr, own);
      break;
    case 'to':
    case 'cc':
      match = email[tree.op].some((recipient) => person(tree.value, '', recipient, own));
      break;
    case 'subject':
      match = has(email.subject);
      break;
    case 'body':
      match = has(email.body);
      break;
    case 'filename':
      match = email.attachments.some((file) => has(file.name));
      break;
    case 'type':
      match = email.attachments.some(
        (file) =>
          normalize(file.name).endsWith(`.${needle}`) ||
          normalize(file.mime).split('/')[1] === needle,
      );
      break;
    case 'label':
      match = email.labels.some((label) => normalize(label) === needle);
      break;
    case 'category':
      match = normalize(email.category) === needle;
      break;
    case 'account':
      match = store.accounts.some(
        (account) =>
          account.id === email.account_id &&
          [account.name, account.email].some((value) => normalize(value) === needle),
      );
      break;
    case 'in':
      match =
        needle === 'anywhere' ||
        (needle === 'snoozed'
          ? (email.snoozed_until || 0) > now
          : normalize(email.folder) === needle);
      break;
    case 'has':
      match =
        needle === 'reminder'
          ? !!email.reminder_at
          : needle === 'attachment'
            ? email.attachments.length > 0
            : email.attachments.some((file) => {
                const ext = file.name.toLowerCase().split('.').at(-1) || '';
                return (
                  needle === 'document'
                    ? ['pdf', 'docx', 'doc', 'txt']
                    : needle === 'spreadsheet'
                      ? ['xlsx', 'xls', 'csv']
                      : ['pptx', 'key']
                ).includes(ext);
              });
      break;
    case 'is':
      match = (
        {
          unread: !email.read,
          read: email.read,
          starred: !!metadata.starred,
          pinned: email.pinned,
          unreplied: !metadata.replied,
          done: email.done,
          snoozed: (email.snoozed_until || 0) > now,
          task:
            !email.done &&
            email.folder !== 'Trash' &&
            !!(email.pinned || email.due || email.reminder_at || email.set_aside),
        } as Record<string, boolean>
      )[needle];
      break;
    case 'newer_than':
      match = !email.date_missing && email.date >= now - tree.duration!;
      break;
    case 'older_than':
      match = !email.date_missing && email.date < now - tree.duration!;
      break;
    default: {
      const date = dateKey(email.date);
      match =
        !email.date_missing &&
        (!tree.lower || date >= tree.lower) &&
        (!tree.upper || date <= tree.upper);
    }
  }
  return [match, tree.op === 'in' && ['anywhere', 'trash', 'spam'].includes(needle)];
}

function atoms(tree: Node): Atom[] {
  if (tree.kind === 'atom') return [tree];
  if (tree.kind === 'not') return [];
  return [...atoms(tree.left), ...atoms(tree.right)];
}

function checkDateIntersection(tree: Node): { lower?: string; upper?: string } {
  if (tree.kind === 'atom') return { lower: tree.lower, upper: tree.upper };
  if (tree.kind === 'not') return {};
  const left = checkDateIntersection(tree.left);
  const right = checkDateIntersection(tree.right);
  if (tree.kind === 'or') return {};
  const lower = [left.lower, right.lower].filter(Boolean).sort().at(-1);
  const upper = [left.upper, right.upper].filter(Boolean).sort()[0];
  if (lower && upper && lower > upper)
    error('Questi periodi non si sovrappongono.', 'These date periods do not overlap.');
  return { lower, upper };
}

export function sampleSearch(
  store: Store,
  text: string,
  accountId: number | null,
  sort = 'relevance',
): { emails: Email[]; terms: string[] } {
  configureRegional(store.settings);
  const tree = parse(text, store);
  checkDateIntersection(tree);
  const filters = atoms(tree);
  const terms = filters
    .filter((filter) => !filter.op || ['subject', 'body'].includes(filter.op))
    .map((filter) => filter.value);
  const own = store.accounts
    .filter((account) => !accountId || account.id === accountId)
    .map((account) => normalize(account.email));
  const now = Date.now() / 1000;
  const emails = store.emails.filter((email) => {
    if (accountId && email.account_id !== accountId) return false;
    const [matches, special] = evaluate(tree, email, store, own, now);
    return matches && (!['spam', 'trash'].includes(normalize(email.folder)) || special);
  });
  const center = filters.find((filter) => filter.center)?.center;
  const distance = (email: Email) =>
    center
      ? Math.abs(Date.parse(`${dateKey(email.date)}T12:00:00Z`) - Date.parse(`${center}T12:00:00Z`))
      : 0;
  const score = (email: Email) =>
    terms.reduce(
      (sum, term) =>
        sum +
        (normalize(email.subject).includes(normalize(term)) ? 5 : 0) +
        (normalize(email.body).includes(normalize(term)) ? 1 : 0) +
        (normalize(email.from_name).includes(normalize(term)) ? 3 : 0),
      0,
    );
  emails.sort((a, b) => {
    const relevance =
      sort !== 'date' && sort !== 'closest' && terms.length ? score(b) - score(a) : 0;
    const proximity = center && sort !== 'date' ? distance(a) - distance(b) : 0;
    return relevance || proximity || b.date - a.date || b.id - a.id;
  });
  return { emails, terms };
}

export function sampleSnippet(
  email: Email,
  terms: string[],
): { snippet: string; match_reason: string } {
  const textSources = [
    { text: email.body, name: '' },
    ...email.attachments.map((file) => ({ text: file.inline_text || '', name: file.name })),
    { text: email.subject, name: 'Oggetto / Subject' },
  ];
  for (const source of textSources) {
    const term = terms.find((term) => normalize(source.text).includes(normalize(term)));
    if (!term) continue;
    const index = normalize(source.text).indexOf(normalize(term));
    return {
      snippet: source.text
        .slice(Math.max(0, index - 45), index + term.length + 95)
        .replace(/\s+/g, ' '),
      match_reason: source.name,
    };
  }
  return { snippet: email.body.replace(/\s+/g, ' ').slice(0, 140), match_reason: '' };
}
