export function searchItalian(locale: string): boolean {
  return locale.toLowerCase().startsWith('it');
}

export function searchText(locale: string, it: string, en: string): string {
  return searchItalian(locale) ? it : en;
}

export function searchErrorMessage(locale: string, message: string): string {
  if (!searchItalian(locale)) return message;
  const translations: [RegExp, string][] = [
    [
      /^Invalid date (.+)\. Use YYYY-MM-DD with a complete year$/,
      'Data non valida $1. Usa AAAA-MM-GG con l’anno completo.',
    ],
    [/^Invalid date: (.+)$/, 'Data non valida: $1'],
    [
      /^Unknown filter (.+)\. Put literal text in double quotes$/,
      'Filtro sconosciuto $1. Racchiudi il testo letterale tra virgolette.',
    ],
    [/^Unknown (?:filter|operator): (.+)$/, 'Filtro sconosciuto: $1'],
    [/^Unknown attachment filter (.+)$/, 'Filtro allegato sconosciuto $1'],
    [/^Unknown message state (.+)$/, 'Stato del messaggio sconosciuto $1'],
    [/^Unknown category (.+)$/, 'Categoria sconosciuta $1'],
    [/^Unknown folder (.+)$/, 'Cartella sconosciuta $1'],
    [/^Unknown account (.+)$/, 'Account sconosciuto $1'],
    [/^Enter a value after (.+)$/, 'Inserisci un valore dopo $1'],
    [/^Close the quoted phrase with a double quote$/, 'Chiudi la frase con le virgolette.'],
    [/^Close the search group with \)$/, 'Chiudi il gruppo di ricerca con ).'],
    [/^Enter a term after the operator$/, 'Inserisci un termine dopo l’operatore.'],
    [
      /^Enter a term before this operator or parenthesis$/,
      'Inserisci un termine prima dell’operatore o della parentesi.',
    ],
    [/^Enter a word or a filter value$/, 'Inserisci una parola o il valore del filtro.'],
    [/^The start date is after the end date$/, 'La data iniziale è successiva a quella finale.'],
    [/^These date periods do not overlap$/, 'Questi periodi non si sovrappongono.'],
    [
      /^The radius must be in days, for example \/6d$/,
      'Il raggio va espresso in giorni, per esempio /6d.',
    ],
    [
      /^Enter a whole number of days from 0 to 365$/,
      'Inserisci un numero intero di giorni da 0 a 365.',
    ],
    [
      /^The radius must be between 0 and 365 days$/,
      'Il raggio deve essere compreso tra 0 e 365 giorni.',
    ],
    [
      /^Use around:YYYY-MM-DD or around:YYYY-MM-DD\/6d$/,
      'Usa around:AAAA-MM-GG oppure around:AAAA-MM-GG/6d.',
    ],
    [/^Use between:YYYY-MM-DD\.\.YYYY-MM-DD$/, 'Usa between:AAAA-MM-GG..AAAA-MM-GG.'],
    [/^Date is out of range$/, 'La data non rientra nell’intervallo supportato.'],
    [
      /^Invalid duration; use a positive whole number and d, w, m or y$/,
      'Durata non valida: usa un intero positivo e d, w, m o y.',
    ],
    [/^Duration unit must be d, w, m or y$/, 'L’unità della durata deve essere d, w, m o y.'],
    [/^Duration cannot be negative$/, 'La durata non può essere negativa.'],
    [/^Duration is too large$/, 'La durata è troppo grande.'],
    [/^Unexpected closing parenthesis$/, 'Parentesi di chiusura inattesa.'],
    [/^Too many search terms$/, 'La ricerca contiene troppi termini.'],
    [/^Search groups are nested too deeply$/, 'La ricerca contiene troppi gruppi annidati.'],
  ];
  for (const [pattern, translated] of translations)
    if (pattern.test(message)) return message.replace(pattern, translated);
  return message;
}

export function quoteSearch(value: string): string {
  return /[\s():"\\]/.test(value) || /^(AND|OR)$/.test(value)
    ? `"${value.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"`
    : value;
}

// Split only at the top level: a quoted phrase or an OR group stays intact.
export function queryParts(query: string): string[] {
  const parts: string[] = [];
  let start = -1;
  let depth = 0;
  let quoted = false;
  let escaped = false;
  for (let i = 0; i < query.length; i++) {
    const char = query[i];
    if (start === -1 && !/\s/.test(char)) start = i;
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === '\\' && quoted) {
      escaped = true;
      continue;
    }
    if (char === '"') quoted = !quoted;
    if (!quoted) {
      if (char === '(') depth++;
      if (char === ')') depth--;
      if (/\s/.test(char) && depth === 0 && start !== -1) {
        parts.push(query.slice(start, i));
        start = -1;
      }
    }
  }
  if (start !== -1) parts.push(query.slice(start));
  return parts;
}

export function hasBooleanGroup(query: string): boolean {
  return queryParts(query).some((part) => part === 'OR' || part === 'AND' || part.startsWith('('));
}

export function withSearchFilter(query: string, operator: string, value: string): string {
  const token = value.trim() ? `${operator}:${quoteSearch(value.trim())}` : '';
  if (queryParts(query).some((part) => part === 'OR' || part === 'AND'))
    return token ? `(${query.trim()}) ${token}` : query.trim();
  const parts = queryParts(query).filter((part) => !part.toLowerCase().startsWith(`${operator}:`));
  if (token) parts.push(token);
  return parts.join(' ');
}

export function filterValue(query: string, operator: string): string {
  const token = queryParts(query).find((part) => part.toLowerCase().startsWith(`${operator}:`));
  const value = token?.slice(operator.length + 1) || '';
  return value.startsWith('"') && value.endsWith('"')
    ? value.slice(1, -1).replace(/\\(["\\])/g, '$1')
    : value;
}

export function removeSearchChip(query: string, index: number): string {
  if (hasBooleanGroup(query)) return '';
  return queryParts(query)
    .filter((_, at) => at !== index)
    .join(' ');
}

export function searchChips(query: string, locale: string): { text: string; operator: string }[] {
  if (hasBooleanGroup(query)) return [{ text: query, operator: 'query' }];
  const labels: Record<string, [string, string]> = {
    from: ['Da', 'From'],
    to: ['A', 'To'],
    cc: ['In copia', 'Cc'],
    subject: ['Oggetto', 'Subject'],
    body: ['Testo', 'Body'],
    on: ['Il giorno', 'On'],
    between: ['Dal… al…', 'Between'],
    around: ['Intorno al', 'Around'],
    after: ['Dal giorno', 'From date'],
    before: ['Prima del', 'Before'],
    has: ['Contiene', 'Has'],
    type: ['Tipo', 'Type'],
    filename: ['File', 'File'],
    in: ['Cartella', 'Folder'],
    folder: ['Cartella', 'Folder'],
    account: ['Account', 'Account'],
    is: ['Stato', 'State'],
    label: ['Etichetta', 'Label'],
    category: ['Categoria', 'Category'],
  };
  return queryParts(query).map((part) => {
    const match = /^([a-z_]+):(.*)$/i.exec(part);
    if (!match) return { text: part, operator: '' };
    const label = labels[match[1]];
    const value = match[2].replace(/^"|"$/g, '');
    return {
      text: `${label ? searchText(locale, ...label) : match[1]}: ${value}`,
      operator: match[1],
    };
  });
}

export function todayISO(timeZone: string): string {
  const parts = new Intl.DateTimeFormat('en-CA', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).formatToParts(new Date());
  const get = (type: string) => parts.find((part) => part.type === type)?.value;
  return `${get('year')}-${get('month')}-${get('day')}`;
}

export function offsetISO(day: string, offset: number): string {
  const date = new Date(`${day}T12:00:00Z`);
  if (!Number.isFinite(date.valueOf())) return '';
  date.setUTCDate(date.getUTCDate() + offset);
  return date.toISOString().slice(0, 10);
}

export function calendarLabel(day: string, locale: string): string {
  const date = new Date(`${day}T12:00:00Z`);
  if (!Number.isFinite(date.valueOf())) return day;
  return new Intl.DateTimeFormat(locale, {
    timeZone: 'UTC',
    day: 'numeric',
    month: 'long',
    year: 'numeric',
  }).format(date);
}

export function canonicalSearchDate(value: string, locale = 'en-US'): string | null {
  let year: string, month: string, day: string;
  const canonical = /^(\d{4})[-/](\d{2})[-/](\d{2})$/.exec(value);
  if (canonical) [, year, month, day] = canonical;
  else {
    const regional = /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(value);
    if (!regional) return null;
    year = regional[3];
    const fields = new Intl.DateTimeFormat(locale)
      .formatToParts(new Date('2026-10-15T12:00:00Z'))
      .filter((part) => part.type === 'month' || part.type === 'day');
    const monthFirst = fields[0]?.type === 'month';
    month = regional[monthFirst ? 1 : 2].padStart(2, '0');
    day = regional[monthFirst ? 2 : 1].padStart(2, '0');
  }
  const iso = `${year}-${month}-${day}`;
  const date = new Date(`${iso}T12:00:00Z`);
  return Number.isFinite(date.valueOf()) && date.toISOString().slice(0, 10) === iso ? iso : null;
}

export function canonicalSearchQuery(query: string, locale: string): string {
  return queryParts(query)
    .map((part) => {
      if (part.startsWith('(') && part.endsWith(')'))
        return `(${canonicalSearchQuery(part.slice(1, -1), locale)})`;
      const token = /^(on|before|after|between|around):(.+)$/.exec(part);
      if (!token) return part;
      const [operator, value] = [token[1], token[2]];
      if (operator === 'between') {
        const dates = value.split('..').map((date) => canonicalSearchDate(date, locale));
        return dates.length === 2 && dates.every(Boolean) ? `between:${dates.join('..')}` : part;
      }
      if (operator === 'around') {
        const range = aroundRange(part, locale);
        return range ? `around:${range.day}/${range.radius}d` : part;
      }
      const day = canonicalSearchDate(value, locale);
      return day ? `${operator}:${day}` : part;
    })
    .join(' ');
}

export function aroundRange(
  query: string,
  locale = 'en-US',
): { day: string; radius: number; start: string; end: string } | null {
  const raw = filterValue(query, 'around');
  const match = /^(.*)\/(\d+)d$/.exec(raw);
  const day = canonicalSearchDate(match ? match[1] : raw, locale);
  const radius = Number(match?.[2] ?? 6);
  if (!day || !Number.isInteger(radius) || radius < 0 || radius > 365) return null;
  return { day, radius, start: offsetISO(day, -radius), end: offsetISO(day, radius) };
}

export interface RecoverySuggestion {
  label: string;
  query: string;
}
export function recoverySuggestions(
  query: string,
  locale: string,
  accountLimited: boolean,
  language: string = locale,
): RecoverySuggestion[] {
  const suggestions: RecoverySuggestion[] = [];
  const around = aroundRange(query, locale);
  if (around && around.radius < 365) {
    const radius = Math.min(365, around.radius < 14 ? 14 : around.radius * 2);
    suggestions.push({
      label: searchText(
        language,
        `Prova ${radius} giorni prima e dopo`,
        `Try ${radius} days before and after`,
      ),
      query: withSearchFilter(query, 'around', `${around.day}/${radius}d`),
    });
  }
  const on = filterValue(query, 'on');
  if (on)
    suggestions.push({
      label: searchText(
        language,
        'Cerca anche nei 6 giorni prima e dopo',
        'Search 6 days before and after too',
      ),
      query: withSearchFilter(withSearchFilter(query, 'on', ''), 'around', `${on}/6d`),
    });
  const interval = filterValue(query, 'between').split('..');
  if (!hasBooleanGroup(query) && interval.length === 2) {
    const start = canonicalSearchDate(interval[0], locale);
    const end = canonicalSearchDate(interval[1], locale);
    if (start && end) suggestions.push({
      label: searchText(language, 'Allarga il periodo di 7 giorni per lato', 'Expand the range by 7 days on each side'),
      query: withSearchFilter(query, 'between', `${offsetISO(start, -7)}..${offsetISO(end, 7)}`),
    });
  }
  const from = filterValue(query, 'from');
  if (from && !hasBooleanGroup(query)) {
    const rest = withSearchFilter(query, 'from', '');
    const person = quoteSearch(from);
    suggestions.push({
      label: searchText(
        language,
        'Cerca la persona anche tra i destinatari',
        'Look for the person among recipients too',
      ),
      query: `${rest} (from:${person} OR to:${person} OR cc:${person})`.trim(),
    });
  }
  const subject = filterValue(query, 'subject');
  if (subject && !hasBooleanGroup(query)) {
    const rest = withSearchFilter(query, 'subject', '');
    const text = quoteSearch(subject);
    suggestions.push({
      label: searchText(
        language,
        'Cerca le parole anche nel testo',
        'Look for these words in the body too',
      ),
      query: `${rest} (subject:${text} OR body:${text})`.trim(),
    });
  }
  if (!hasBooleanGroup(query) && filterValue(query, 'in') !== 'anywhere') {
    suggestions.push({
      label: searchText(language, 'Includi Spam e Cestino', 'Include Spam and Trash'),
      query: withSearchFilter(query, 'in', 'anywhere'),
    });
  }
  if (!hasBooleanGroup(query) && (accountLimited || filterValue(query, 'account')))
    suggestions.push({
      label: searchText(language, 'Cerca in tutti gli account', 'Search all accounts'),
      query: withSearchFilter(query, 'account', ''),
    });
  return suggestions.slice(0, 3);
}

export function markedParts(text: string, terms: string[]): { text: string; match: boolean }[] {
  const escaped = terms.filter(Boolean).map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  if (!escaped.length) return [{ text, match: false }];
  const expression = new RegExp(`(${escaped.join('|')})`, 'giu');
  return text.split(expression).map((part, index) => ({ text: part, match: index % 2 === 1 }));
}
