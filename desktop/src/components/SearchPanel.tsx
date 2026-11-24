// What opens under the search field when it is clicked or reached with Ctrl K: the filters that take one click (unread, with an
// attachment, today, the last seven days, pinned, to do), the searches run last, the people to write «da:» for, and, behind
// «Filtri avanzati», the form with every filter. The query is the one source: a quick filter reads it to know whether it is on
// and writes it when it is pressed. Recent searches and people show while the field is empty; typing leaves the quick filters.
import { useEffect, useState } from 'react';
import { call } from '../lib/api';
import { tr } from '../lib/i18n';
import { filterValue, offsetISO, recentSearches, todayISO, withSearchFilter } from '../lib/search';
import Avatar from '../ui/Avatar';
import Button from '../ui/Button';
import Icon from '../ui/Icon';
import SearchFilters, { type Person } from './SearchFilters';

export type SearchPanelProps = {
  query: string;
  locale: string;
  zone: string;
  accountId: number | null;
  /** The accounts and the labels the form of the filters chooses among. */
  accounts: { value: string; label: string }[];
  labels: string[];
  sort: string;
  closest: boolean;
  /** Open the form of every filter from the start (a chip of the query was pressed). */
  advanced: boolean;
  /** `keepOpen`: a quick filter leaves the panel open, to press another. */
  onapply: (query: string, keepOpen?: boolean) => void;
  onsort: (sort: string) => void;
};

export default function SearchPanel({ query, locale, zone, accountId, accounts, labels, sort, closest, advanced, onapply, onsort }: SearchPanelProps) {
  const [more, setMore] = useState(advanced);
  const [people, setPeople] = useState<Person[]>([]);
  const t = tr;
  const recent = recentSearches();
  const today = todayISO(zone);
  const week = `${offsetISO(today, -6)}..${today}`;

  useEffect(() => {
    let current = true;
    (async () => {
      try {
        let result: Person[];
        try {
          // All accounts: the unified address book. One account: people seen in that mailbox.
          if (accountId !== null) throw new Error('mailbox scope');
          const list = await call<{ items: { fields: { name: string; emails: { value: string }[] } }[] }>('contacts_list', {
            query: { text: '', book: null, include_hidden: false, offset: 0, limit: 500 },
          });
          result = list.items.flatMap((c) => c.fields.emails.map((e) => ({ name: c.fields.name, address: e.value })));
        } catch {
          result = await call<Person[]>('search_suggestions', { accountId });
        }
        if (current) setPeople(result);
      } catch {
        if (current) setPeople([]);
      }
    })();
    return () => {
      current = false;
    };
  }, [accountId]);

  const dateOps = ['on', 'between', 'around', 'after', 'before'];
  const withoutDates = (value: string) => dateOps.reduce((result, op) => withSearchFilter(result, op, ''), value);
  const quick: { label: string; on: boolean; toggle: () => string }[] = [
    {
      label: t('Non letti', 'Unread'),
      on: filterValue(query, 'is') === 'unread',
      toggle: () => withSearchFilter(query, 'is', filterValue(query, 'is') === 'unread' ? '' : 'unread'),
    },
    {
      label: t('Con allegato', 'With attachment'),
      on: filterValue(query, 'has') === 'attachment',
      toggle: () => withSearchFilter(query, 'has', filterValue(query, 'has') === 'attachment' ? '' : 'attachment'),
    },
    {
      label: t('Oggi', 'Today'),
      on: filterValue(query, 'on') === today,
      toggle: () => (filterValue(query, 'on') === today ? withoutDates(query) : withSearchFilter(withoutDates(query), 'on', today)),
    },
    {
      label: t('Ultimi 7 giorni', 'Last 7 days'),
      on: filterValue(query, 'between') === week,
      toggle: () => (filterValue(query, 'between') === week ? withoutDates(query) : withSearchFilter(withoutDates(query), 'between', week)),
    },
    {
      label: t('Fissati', 'Pinned'),
      on: filterValue(query, 'is') === 'pinned',
      toggle: () => withSearchFilter(query, 'is', filterValue(query, 'is') === 'pinned' ? '' : 'pinned'),
    },
    {
      label: t('Da fare', 'To do'),
      on: filterValue(query, 'is') === 'task',
      toggle: () => withSearchFilter(query, 'is', filterValue(query, 'is') === 'task' ? '' : 'task'),
    },
  ];
  // The caption of a block, as the sidebar's group labels: 12px, semibold, spaced. A paragraph, not an h3, so the base titles' size does not reach it.
  const heading = 'text-xs font-semibold uppercase tracking-wider text-ink-mute';
  const idle = !query.trim();
  const frequent = people.slice(0, 6);

  return (
    <div className="flex flex-col gap-3">
      <section aria-label={t('Filtri rapidi', 'Quick filters')} className="flex flex-col gap-1.5">
        <p className={heading}>{t('Filtri rapidi', 'Quick filters')}</p>
        <div className="flex flex-wrap gap-1.5">
          {quick.map((item) => (
            <Button key={item.label} type="button" variant={item.on ? 'primary' : 'secondary'} aria-pressed={item.on} onClick={() => onapply(item.toggle(), true)}>
              {item.label}
            </Button>
          ))}
        </div>
      </section>
      {idle && recent.length > 0 && (
        <section aria-label={t('Ricerche recenti', 'Recent searches')} className="flex flex-col gap-1">
          <p className={heading}>{t('Recenti', 'Recent')}</p>
          <ul role="list" className="grid grid-cols-2 gap-x-2">
            {recent.map((item) => (
              <li key={item}>
                <button
                  type="button"
                  onClick={() => onapply(item)}
                  className="flex h-(--control-sm) w-full items-center gap-2 rounded-md px-2 text-start text-sm text-ink-soft hover:bg-selected/60 hover:text-ink"
                >
                  <Icon name="history" size={14} />
                  <span className="truncate">{item}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {idle && frequent.length > 0 && (
        <section aria-label={t('Persone', 'People')} className="flex flex-col gap-1.5">
          <p className={heading}>{t('Persone', 'People')}</p>
          <div className="flex flex-wrap gap-1.5">
            {frequent.map((person) => (
              <button
                key={person.address}
                type="button"
                onClick={() => onapply(withSearchFilter(query, 'from', person.address))}
                className="flex h-(--control-sm) items-center gap-2 rounded-md border border-line px-2 text-sm text-ink-soft hover:bg-selected/60 hover:text-ink"
              >
                <Avatar name={person.name || person.address} size="xs" decorative />
                <span className="max-w-40 truncate">{person.name || person.address}</span>
              </button>
            ))}
          </div>
        </section>
      )}
      <div className="flex flex-col gap-2 border-t border-line pt-2">
        <Button type="button" variant="text" aria-expanded={more} onClick={() => setMore(!more)} className="self-start">
          {t('Filtri avanzati', 'Advanced filters')}
          <Icon name="down" size={12} />
        </Button>
        {more && <SearchFilters query={query} locale={locale} zone={zone} people={people} accounts={accounts} labels={labels} sort={sort} closest={closest} onapply={onapply} onsort={onsort} />}
      </div>
    </div>
  );
}
