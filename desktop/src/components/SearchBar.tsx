import { useEffect, useRef, type KeyboardEvent } from 'react';
import { calendarWords as dateWords } from '../lib/calendar-words';
import Button from '../ds/primitives/Button';
import Tooltip from '../ds/primitives/Tooltip';
import DateField from '../ds/primitives/DateField';
import Icon from '../ds/primitives/Icon';
import Select from '../ds/primitives/Select';
import { call } from '../lib/api';
import { backendText, isItalian, tr } from '../lib/i18n';
import { regionalLocale, regionalTimeZone } from '../lib/regional';
import {
  aroundRange,
  calendarLabel,
  canonicalSearchDate,
  canonicalSearchQuery,
  filterValue,
  hasBooleanGroup,
  offsetISO,
  queryParts,
  quoteSearch,
  recoverySuggestions,
  removeSearchChip,
  searchChips,
  searchErrorMessage,
  todayISO,
  withSearchFilter,
} from '../lib/search';
import { useReactive } from '../lib/store';
import { tick } from '../lib/tick';
import { cn } from '../lib/utils';
import type { MailPage, Workspace } from '../lib/types';
import { imeComposing } from '../ds/ime';
import { Input } from '../ui/Input';

export type SearchBarProps = {
  query: string;
  workspace: Workspace;
  accountId: number | null;
  page: MailPage;
  loading: boolean;
  error: string;
  displayedQuery: string;
  sort: string;
  remoteBusy: boolean;
  /** The server has more results for this search. */
  remoteMore: boolean;
  canUndo: boolean;
  onquery: (query: string, recovery?: boolean) => void;
  onsort: (sort: string) => void;
  onremote: () => void;
  onremotemore: () => void;
  oncancel: () => void;
  onundo: () => void;
  onallaccounts: () => void;
  focusSearch: () => void;
};

type Panel = '' | 'from' | 'to' | 'when' | 'attachments' | 'more' | 'help';

const t = tr;
const fileTypes = ['pdf', 'docx', 'xlsx', 'csv', 'pptx', 'txt', 'jpg', 'png'].map((type) => ({
  value: type,
  label: type.toUpperCase(),
}));

export default function SearchBar({
  query,
  workspace,
  accountId,
  page,
  loading,
  error,
  displayedQuery,
  sort,
  remoteBusy,
  remoteMore,
  canUndo,
  onquery,
  onsort,
  onremote,
  onremotemore,
  oncancel,
  onundo,
  onallaccounts,
  focusSearch,
}: SearchBarProps) {
  // The panel's state is read after awaits (suggestions, tick): it is always current here.
  const s = useReactive(() => ({
    panel: '' as Panel,
    field: '',
    people: [] as string[],
    suggestions: [] as { name: string; address: string }[],
    mode: 'around',
    day: '',
    end: '',
    radius: 6 as number | null,
    recent: '7',
    advanced: 'subject',
    attachment: 'has',
    recovering: false,
    copied: false,
    showOperators: false,
  }));
  const suggestionRequest = useRef(0);
  const firstControl = useRef<HTMLInputElement>(null);
  const autoOpened = useRef('');
  const locale = workspace.settings.locale || regionalLocale();
  const zone = page.search?.timezone || regionalTimeZone();
  // Chip and error wording follow the interface language. Dates still follow the regional format.
  const wordingLocale = isItalian() ? 'it-IT' : 'en-US';
  // The calendar of the date fields speaks the interface language.
  const calendarWords = dateWords();
  const modes = [
    { value: 'around', label: t('Intorno a una data', 'Around a date') },
    { value: 'on', label: t('Giorno preciso', 'Exact day') },
    { value: 'between', label: t('Intervallo', 'Date range') },
    { value: 'recent', label: t('Periodo recente', 'Recent period') },
  ];
  const recents = [
    { value: '0', label: t('Oggi', 'Today') },
    { value: '1', label: t('Ieri', 'Yesterday') },
    { value: '7', label: t('Ultimi 7 giorni', 'Last 7 days') },
    { value: 'month', label: t('Mese scorso', 'Last month') },
  ];
  const attachmentKinds = [
    { value: 'has', label: t('C’è un allegato', 'Has an attachment') },
    { value: 'filename', label: t('Nome del file', 'File name') },
    { value: 'type', label: t('Tipo di file', 'File type') },
  ];
  const fields = [
    { value: 'subject', label: t('Oggetto contiene', 'Subject contains') },
    { value: 'body', label: t('Testo contiene', 'Body contains') },
    { value: 'cc', label: t('Persona in copia', 'Person in Cc') },
    { value: 'in', label: t('Cartella', 'Folder') },
    { value: 'account', label: 'Account' },
    { value: 'is', label: t('Stato', 'State') },
    { value: 'label', label: t('Etichetta', 'Label') },
    { value: 'category', label: t('Categoria', 'Category') },
  ];
  const folders = [
    { value: 'inbox', label: t('Posta in arrivo', 'Inbox') },
    { value: 'sent', label: t('Inviata', 'Sent') },
    { value: 'archive', label: t('Archivio', 'Archive') },
    { value: 'spam', label: 'Spam' },
    { value: 'trash', label: t('Cestino', 'Trash') },
    { value: 'anywhere', label: t('Tutta la posta, inclusi Spam e Cestino', 'All mail, including Spam and Trash') },
  ];
  const states = [
    { value: 'unread', label: t('Da leggere', 'Unread') },
    { value: 'read', label: t('Letta', 'Read') },
    { value: 'starred', label: t('Speciale', 'Starred') },
    { value: 'pinned', label: t('Fissata', 'Pinned') },
    { value: 'snoozed', label: t('Posticipata', 'Snoozed') },
    { value: 'unreplied', label: t('Senza risposta', 'Unreplied') },
    { value: 'done', label: t('Completata', 'Done') },
    { value: 'task', label: t('Da fare', 'Task') },
  ];
  const categories = [
    { value: 'personal', label: t('Personale', 'personal') },
    { value: 'priority', label: t('Priorità', 'priority') },
    { value: 'notification', label: t('Notifica', 'notification') },
    { value: 'newsletter', label: t('Newsletter', 'newsletter') },
    { value: 'invitation', label: t('Invito', 'invitation') },
    { value: 'invitation_response', label: t('Risposta a un invito', 'invitation_response') },
  ];
  const accounts = workspace.accounts.map((item) => ({ value: item.email, label: `${item.name} · ${item.email}` }));
  const chips = searchChips(query, wordingLocale);
  const around = aroundRange(query, locale);
  // The sort has a first choice whose value is '' (whatever suits the search); a Select does not hold
  // an empty value, so that choice is called 'default' here.
  const sorts = [
    {
      value: 'default',
      label:
        around && !queryParts(query).some((part) => !part.includes(':'))
          ? t('Più vicine alla data', 'Closest to date')
          : t('Più pertinenti', 'Most relevant'),
    },
    { value: 'date', label: t('Più recenti', 'Newest first') },
    { value: 'relevance', label: t('Più pertinenti', 'Most relevant') },
    ...(around ? [{ value: 'closest', label: t('Più vicine alla data', 'Closest to date') }] : []),
  ];
  const alternatives = recoverySuggestions(query, locale, accountId !== null, isItalian() ? 'it-IT' : 'en-US');
  const scope = filterValue(query, 'in');
  const account = workspace.accounts.find((item) => item.id === accountId);
  const coverageIncomplete = !!page.search && page.search.coverage !== 'complete';

  async function openPanel(next: Panel) {
    s.panel = s.panel === next ? '' : next;
    if (!s.panel) return;
    s.field = filterValue(query, next === 'attachments' ? 'filename' : next === 'more' ? s.advanced : next);
    s.people = [];
    if (next === 'attachments') {
      s.attachment = filterValue(query, 'type') ? 'type' : filterValue(query, 'filename') ? 'filename' : 'has';
      s.field = filterValue(query, s.attachment);
    }
    if (next === 'when') {
      const current = aroundRange(query, locale);
      s.mode = current
        ? 'around'
        : filterValue(query, 'between')
          ? 'between'
          : filterValue(query, 'on')
            ? 'on'
            : 'around';
      s.day =
        current?.day ||
        canonicalSearchDate(filterValue(query, 'on'), locale) ||
        canonicalSearchDate(filterValue(query, 'between').split('..')[0], locale) ||
        todayISO(zone);
      s.end = canonicalSearchDate(filterValue(query, 'between').split('..')[1], locale) || s.day;
      s.radius = current?.radius ?? 6;
    }
    if (next === 'from' || next === 'to') {
      const request = ++suggestionRequest.current;
      try {
        let result: { name: string; address: string }[];
        try {
          // All accounts: the unified address book. One account: people seen in that mailbox.
          if (accountId !== null) throw new Error('mailbox scope');
          const list = await call<{ items: { fields: { name: string; emails: { value: string }[] } }[] }>(
            'contacts_list',
            { query: { text: '', book: null, include_hidden: false, offset: 0, limit: 500 } },
          );
          result = list.items.flatMap((c) => c.fields.emails.map((e) => ({ name: c.fields.name, address: e.value })));
        } catch {
          result = await call<{ name: string; address: string }[]>('search_suggestions', { accountId });
        }
        if (request === suggestionRequest.current) s.suggestions = result;
      } catch {
        s.suggestions = [];
      }
    }
    await tick();
    (
      firstControl.current ??
      document.querySelector<HTMLElement>(
        '#search-filter-panel form [data-scope="select"][data-part="trigger"], #search-filter-panel form input',
      )
    )?.focus();
  }

  useEffect(() => {
    const match = /(?:^|\s)(from|to):$/.exec(query);
    if (match && autoOpened.current !== query) {
      autoOpened.current = query;
      void openPanel(match[1] as Panel);
    }
    // openPanel reads the current query itself.
  }, [query]);

  function apply(value: string) {
    onquery(value);
    s.panel = '';
    focusSearch();
  }
  function addPerson() {
    const value = s.field.trim();
    if (value && !s.people.includes(value)) s.people = [...s.people, value];
    s.field = '';
    firstControl.current?.focus();
  }
  function applyPeople() {
    const values = [...s.people, s.field.trim()].filter(Boolean);
    if (!values.length) return;
    if (values.length === 1) apply(withSearchFilter(query, s.panel, values[0]));
    else {
      const rest = withSearchFilter(query, s.panel, '');
      const group = `(${values.map((value) => `${s.panel}:${quoteSearch(value)}`).join(' OR ')})`;
      apply(`${hasBooleanGroup(rest) ? `(${rest})` : rest} ${group}`.trim());
    }
  }
  function applyDate() {
    let result = query;
    for (const op of ['on', 'between', 'around', 'after', 'before']) result = withSearchFilter(result, op, '');
    if (s.mode === 'recent') {
      const today = todayISO(zone);
      if (s.recent === '0') result = withSearchFilter(result, 'on', today);
      else if (s.recent === '1') result = withSearchFilter(result, 'on', offsetISO(today, -1));
      else if (s.recent === 'month') {
        const first = `${today.slice(0, 7)}-01`;
        const last = offsetISO(first, -1);
        result = withSearchFilter(result, 'between', `${last.slice(0, 7)}-01..${last}`);
      } else result = withSearchFilter(result, 'between', `${offsetISO(today, -6)}..${today}`);
    } else if (s.mode === 'between') result = withSearchFilter(result, 'between', `${s.day}..${s.end}`);
    else if (s.mode === 'around') result = withSearchFilter(result, 'around', `${s.day}/${s.radius}d`);
    else result = withSearchFilter(result, 'on', s.day);
    apply(result);
  }
  function editChip(index: number) {
    const chip = chips[index];
    const controls: Record<string, Panel> = {
      from: 'from',
      to: 'to',
      around: 'when',
      on: 'when',
      between: 'when',
      filename: 'attachments',
      type: 'attachments',
      has: 'attachments',
    };
    if (controls[chip.operator]) void openPanel(controls[chip.operator]);
    else if (['subject', 'body', 'cc', 'in', 'account', 'is', 'label', 'category'].includes(chip.operator)) {
      s.advanced = chip.operator;
      void openPanel('more');
    } else focusSearch();
  }
  function recover(value: string) {
    onquery(value, true);
    s.recovering = false;
  }
  async function copyQuery() {
    try {
      await navigator.clipboard.writeText(canonicalSearchQuery(query, locale));
      s.copied = true;
      setTimeout(() => (s.copied = false), 1800);
    } catch {
      focusSearch();
    }
  }
  function closePanel() {
    s.panel = '';
    focusSearch();
  }

  const { panel, field, mode, day, end, radius } = s;
  const radiusOk = radius !== null && Number.isInteger(radius) && radius >= 0 && radius <= 365;
  const valueSelect = (placeholder: string, items: { value: string; label: string }[]) => (
    <Select
      label={t('Valore', 'Value')}
      placeholder={placeholder}
      value={field}
      onChange={(value) => (s.field = value)}
      items={items}
    />
  );

  return (
    <section className="search-workbench" aria-label={t('Filtri di ricerca', 'Search filters')}>
      <div className="search-filter-controls">
        {(
          [
            ['from', t('Da', 'From')],
            ['to', t('A', 'To')],
            ['when', t('Quando', 'When')],
            ['attachments', t('Allegati', 'Attachments')],
            ['more', t('Altri filtri', 'More filters')],
          ] as [Panel, string][]
        ).map(([key, label]) => (
          <button
            key={key}
            className={cn('search-filter-button', panel === key && 'active')}
            aria-expanded={panel === key}
            aria-controls="search-filter-panel"
            onClick={() => openPanel(key)}
          >
            {label}
            <Icon name="down" size={12} />
          </button>
        ))}
        <button
          className="search-help-button"
          onClick={() => openPanel('help')}
          aria-label={t('Guida alla ricerca', 'Search guide')}
          title={t('Aiutami a trovarla', 'Help me find it')}
        >
          <Icon name="info" size={15} />
        </button>
        {!!query.trim() && (
          <Select
            className="search-order"
            aria-label={t('Ordina risultati', 'Sort results')}
            value={sort || 'default'}
            items={sorts}
            onChange={(value) => onsort(value === 'default' ? '' : value)}
          />
        )}
      </div>

      {panel && (
        <div
          id="search-filter-panel"
          className="search-filter-panel"
          onKeyDown={(event: KeyboardEvent) => {
            // Escape inside the open calendar closes the calendar, not the filter.
            const calendar = event.target instanceof Element && event.target.closest('[data-part="content"]');
            if (event.key === 'Escape' && !calendar && !imeComposing(event.nativeEvent)) closePanel();
          }}
          role="dialog"
          aria-modal="false"
          tabIndex={-1}
          aria-label={t('Imposta filtro', 'Set filter')}
        >
          <Tooltip text={t('Chiudi filtro', 'Close filter')}>
            <button className="search-panel-close" aria-label={t('Chiudi filtro', 'Close filter')} onClick={closePanel}>
              ×
            </button>
          </Tooltip>
          {panel === 'from' || panel === 'to' ? (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                applyPeople();
              }}
            >
              <label>
                {panel === 'from'
                  ? t('Chi ha inviato l’email?', 'Who sent the email?')
                  : t('A chi era indirizzata?', 'Who was it sent to?')}{' '}
                <Input
                  ref={firstControl}
                  value={field}
                  onChange={(event) => (s.field = event.target.value)}
                  list="search-people"
                  autoComplete="off"
                  placeholder={t('Nome o indirizzo email', 'Name or email address')}
                />
              </label>
              <datalist id="search-people">
                {s.suggestions.map((person, i) => (
                  <option key={`${person.address}:${i}`} value={person.address}>
                    {person.name} · {person.address}
                  </option>
                ))}
              </datalist>
              {!!s.people.length && (
                <p className="search-person-selections">
                  {s.people.join(' · ')} <small>{t('(uno qualsiasi)', '(any of these)')}</small>
                </p>
              )}
              <div className="search-panel-actions">
                <Button type="button" variant="secondary" disabled={!field.trim()} onClick={addPerson}>
                  {t('Aggiungi un’altra persona', 'Add another person')}
                </Button>
                <Button type="submit" variant="primary" disabled={!field.trim() && !s.people.length}>
                  {t('Applica filtro', 'Apply filter')}
                </Button>
              </div>
            </form>
          ) : panel === 'when' ? (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                applyDate();
              }}
            >
              <Select
                label={t('Quanto ricordi della data?', 'What do you remember about the date?')}
                value={mode}
                onChange={(value) => (s.mode = value)}
                items={modes}
              />
              {mode === 'recent' ? (
                <Select
                  label={t('Periodo', 'Period')}
                  value={s.recent}
                  onChange={(value) => (s.recent = value)}
                  items={recents}
                />
              ) : (
                <>
                  <div className="search-date-fields">
                    <div className="search-field">
                      <span>{mode === 'between' ? t('Dal giorno', 'From date') : t('Data', 'Date')}</span>
                      <DateField
                        clearable
                        value={day}
                        onChange={(value) => (s.day = value)}
                        aria-label={mode === 'between' ? t('Dal giorno', 'From date') : t('Data', 'Date')}
                        locale={locale}
                        translations={calendarWords}
                      />
                    </div>
                    {mode === 'between' && (
                      <div className="search-field">
                        <span>{t('Al giorno incluso', 'Through date, inclusive')}</span>
                        <DateField
                          clearable
                          value={end}
                          onChange={(value) => (s.end = value)}
                          aria-label={t('Al giorno incluso', 'Through date, inclusive')}
                          min={day}
                          locale={locale}
                          translations={calendarWords}
                        />
                      </div>
                    )}
                    {mode === 'around' && (
                      <label>
                        {t('Giorni prima e dopo', 'Days before and after')}
                        <Input
                          type="number"
                          min="0"
                          max="365"
                          step="1"
                          value={radius ?? ''}
                          onChange={(event) =>
                            (s.radius = event.target.value === '' ? null : event.target.valueAsNumber)
                          }
                          required
                        />
                      </label>
                    )}
                  </div>
                  {mode === 'around' && (
                    <>
                      <div className="search-radius-presets">
                        {[3, 6, 14, 30].map((value) => (
                          <button
                            key={value}
                            type="button"
                            className={cn(radius === value && 'chosen')}
                            aria-pressed={radius === value}
                            onClick={() => (s.radius = value)}
                          >
                            ±{value}
                          </button>
                        ))}
                      </div>
                      {day && radiusOk && (
                        <p className="search-date-preview">
                          {calendarLabel(offsetISO(day, -radius), locale)} —{' '}
                          {calendarLabel(offsetISO(day, radius), locale)}
                          <small>
                            {`${radius} ${
                              radius === 1
                                ? t('giorno prima e dopo', 'day before and after')
                                : t('giorni prima e dopo', 'days before and after')
                            } · ${radius * 2 + 1} ${
                              radius === 0 ? t('giorno incluso', 'day included') : t('giorni inclusi', 'days included')
                            }`}
                          </small>
                        </p>
                      )}
                    </>
                  )}
                </>
              )}
              <p className="search-filter-hint">
                {t('Fuso orario', 'Time zone')}: {zone}
              </p>
              <div className="search-panel-actions">
                {mode === 'around' && (
                  <Button
                    variant="secondary"
                    type="button"
                    onClick={() => {
                      const r = s.radius ?? 0;
                      s.end = offsetISO(s.day, r);
                      s.day = offsetISO(s.day, -r);
                      s.mode = 'between';
                    }}
                  >
                    {t('Scegli un intervallo diverso', 'Choose a different range')}
                  </Button>
                )}
                <Button type="submit" variant="primary" disabled={mode !== 'recent' && (!day || (mode === 'between' && !end))}>
                  {t('Applica filtro', 'Apply filter')}
                </Button>
              </div>
            </form>
          ) : panel === 'attachments' ? (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                if (s.attachment === 'type' && !s.field) return;
                apply(withSearchFilter(query, s.attachment, s.attachment === 'has' ? 'attachment' : s.field));
              }}
            >
              <Select
                label={t('Che cosa ricordi dell’allegato?', 'What do you remember about the attachment?')}
                value={s.attachment}
                onChange={(value) => {
                  s.attachment = value;
                  s.field = '';
                }}
                items={attachmentKinds}
              />
              {s.attachment === 'type' ? (
                <Select
                  label={t('Tipo', 'Type')}
                  placeholder={t('Scegli un tipo', 'Choose a type')}
                  value={field}
                  onChange={(value) => (s.field = value)}
                  items={fileTypes}
                />
              ) : s.attachment === 'filename' ? (
                <label>
                  {t('Il nome contiene', 'Name contains')}
                  <Input
                    ref={firstControl}
                    value={field}
                    onChange={(event) => (s.field = event.target.value)}
                    placeholder={t('Per esempio: preventivo', 'For example: proposal')}
                    required
                  />
                </label>
              ) : null}
              <div className="search-panel-actions">
                <Button type="submit" variant="primary" disabled={s.attachment === 'type' && !field}>
                  {t('Applica filtro', 'Apply filter')}
                </Button>
              </div>
            </form>
          ) : panel === 'more' ? (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                if (s.field.trim()) apply(withSearchFilter(query, s.advanced, s.field));
              }}
            >
              <Select
                label={t('Cerca per', 'Search by')}
                value={s.advanced}
                onChange={(value) => {
                  s.advanced = value;
                  s.field = filterValue(query, value);
                }}
                items={fields}
              />
              {s.advanced === 'in' ? (
                valueSelect(t('Scegli una cartella', 'Choose a folder'), folders)
              ) : s.advanced === 'account' ? (
                valueSelect(t('Scegli un account', 'Choose an account'), accounts)
              ) : s.advanced === 'is' ? (
                valueSelect(t('Scegli uno stato', 'Choose a state'), states)
              ) : s.advanced === 'category' ? (
                valueSelect(t('Scegli una categoria', 'Choose a category'), categories)
              ) : (
                <label>
                  {t('Valore', 'Value')}{' '}
                  <Input
                    ref={firstControl}
                    value={field}
                    onChange={(event) => (s.field = event.target.value)}
                    list={s.advanced === 'label' ? 'search-labels' : undefined}
                    required
                  />
                </label>
              )}
              <datalist id="search-labels">
                {workspace.labels.map((label) => (
                  <option key={label} value={label}></option>
                ))}
              </datalist>
              <div className="search-panel-actions">
                <Button type="submit" variant="primary" disabled={!field.trim()}>
                  {t('Applica filtro', 'Apply filter')}
                </Button>
              </div>
            </form>
          ) : panel === 'help' ? (
            <>
              <h3>{t('Che cosa ricordi?', 'What do you remember?')}</h3>
              <p>{t('Basta un indizio. Aggiungi un filtro solo se serve.', 'One clue is enough. Add a filter only if needed.')}</p>
              <div className="search-guide-options">
                {(
                  [
                    ['from', t('Una persona', 'A person')],
                    ['when', t('Un periodo', 'A time period')],
                    ['word', t('Una parola', 'A word')],
                    ['attachments', t('Un allegato', 'An attachment')],
                  ] as [string, string][]
                ).map(([key, label]) => (
                  <Button
                    key={key}
                    variant="secondary"
                    onClick={() => {
                      if (key === 'word') closePanel();
                      else void openPanel(key as Panel);
                    }}
                  >
                    {label}
                  </Button>
                ))}
              </div>
              <ol className="search-guide">
                <li>{t('Scrivi ciò che ricordi.', 'Type what you remember.')}</li>
                <li>{t('Aggiungi una data o una persona se serve.', 'Add a date or person if needed.')}</li>
                <li>{t('Apri un risultato riconoscibile.', 'Open a result you recognize.')}</li>
                <li>{t('Se manca, premi “Non la trovo”.', 'If it is missing, choose “I can’t find it”.')}</li>
              </ol>
            </>
          ) : null}
        </div>
      )}

      {!!query.trim() && (
        <>
          <div className="search-applied-filters" aria-label={t('Filtri applicati', 'Applied filters')}>
            {chips.map((chip, index) => (
              <span key={`${index}:${chip.text}`} className="search-chip">
                <button onClick={() => editChip(index)} title={t('Modifica filtro', 'Edit filter')}>
                  {chip.text}
                </button>
                <Tooltip text={`${t('Rimuovi', 'Remove')} ${chip.text}`}>
                  <button
                    className="search-chip-remove"
                    aria-label={`${t('Rimuovi', 'Remove')} ${chip.text}`}
                    onClick={() => onquery(removeSearchChip(query, index))}
                  >
                    <Icon name="close" size={13} />
                  </button>
                </Tooltip>
              </span>
            ))}{' '}
            <Button variant="text" onClick={() => onquery('')}>
              {t('Cancella ricerca', 'Clear search')}
            </Button>{' '}
            <Button variant="text" onClick={() => (s.showOperators = !s.showOperators)}>
              {s.showOperators ? t('Nascondi operatori', 'Hide operators') : t('Mostra operatori', 'Show operators')}
            </Button>
          </div>
          {s.showOperators && (
            <div className="search-query-code">
              <code>{canonicalSearchQuery(query, locale)}</code>
              <Button variant="secondary" onClick={copyQuery}>
                {s.copied ? t('Copiata', 'Copied') : t('Copia ricerca', 'Copy search')}
              </Button>
            </div>
          )}
          <p className="search-scope">
            {t('Ambito', 'Scope')}: {filterValue(query, 'account') || account?.name || t('tutti gli account', 'all accounts')}{' '}
            · {scope || t('tutta la posta, esclusi Spam e Cestino', 'all mail, except Spam and Trash')} · {zone}
            {around && (
              <span>
                {calendarLabel(around.start, locale)} — {calendarLabel(around.end, locale)}
              </span>
            )}
          </p>
        </>
      )}

      {error && (
        <div className="search-error" role="alert">
          <strong>{t('Correggi la ricerca', 'Check your search')}</strong>
          <span>{searchErrorMessage(wordingLocale, error)}</span>
          <small>
            {t('Sono ancora visibili i risultati dell’ultima ricerca valida', 'Showing the last valid search results')}:{' '}
            {displayedQuery || t('elenco precedente', 'previous message list')}
          </small>
          <Button variant="secondary" onClick={() => onquery(quoteSearch(query))}>
            {t('Cerca come testo', 'Search as text')}
          </Button>
        </div>
      )}
      {loading && !!query && (
        <p className="search-progress" role="status">
          {t('Sto cercando…', 'Searching…')}
        </p>
      )}

      {!!query.trim() && !error && (
        <div className="search-recovery">
          {coverageIncomplete && (
            <p className="search-coverage" role="status">
              <strong>{t('Risultati parziali', 'Partial results')}</strong> ·{' '}
              {backendText(
                page.search?.notice ||
                  t('La copertura della posta non è ancora verificata.', 'Mail coverage has not been verified yet.'),
              )}
            </p>
          )}
          {!coverageIncomplete && page.search?.notice && (
            <p className="search-coverage" role="status">
              {backendText(page.search.notice)}
            </p>
          )}
          <div className="search-recovery-actions">
            {coverageIncomplete && page.search?.remote_available && !remoteMore && (
              <Button variant="secondary" disabled={remoteBusy || loading} onClick={onremote}>
                {remoteBusy ? t('Ricerca sul server…', 'Searching server…') : t('Cerca sul server', 'Search server')}
              </Button>
            )}
            {remoteMore && page.search?.remote_available && (
              <Button variant="secondary" disabled={remoteBusy || loading} onClick={onremotemore}>
                {remoteBusy
                  ? t('Ricerca sul server…', 'Searching server…')
                  : t('Altri risultati dal server', 'More results from the server')}
              </Button>
            )}
            {remoteBusy && (
              <Button variant="text" onClick={oncancel}>
                {t('Annulla ricerca sul server', 'Cancel server search')}
              </Button>
            )}{' '}
            <Button variant="text" aria-expanded={s.recovering} onClick={() => (s.recovering = !s.recovering)}>
              {t('Non la trovo', 'I can’t find it')}
            </Button>{' '}
            {canUndo && (
              <Button variant="text" onClick={onundo}>
                {t('Annulla modifica', 'Undo change')}
              </Button>
            )}
          </div>
          {s.recovering && (
            <div className="search-recovery-options">
              <p>
                {!alternatives.length
                  ? t(
                      'Questa ricerca contiene gruppi: modifica una data o un filtro nella barra per mantenere le alternative.',
                      'This search contains groups: edit a date or filter in the search box to preserve its alternatives.',
                    )
                  : coverageIncomplete
                    ? t(
                        'Prima controlla la posta non ancora cercata. Oppure modifica un solo criterio:',
                        'First check mail that has not been searched. Or change one criterion:',
                      )
                    : t('Prova una modifica. Puoi annullarla.', 'Try one change. You can undo it.')}
              </p>
              {alternatives.map((alternative, index) => (
                <Button
                  key={`${index}:${alternative.query}`}
                  variant={index === 0 && !coverageIncomplete ? 'primary' : 'secondary'}
                  onClick={() => {
                    recover(alternative.query);
                    // Recovery labels are built with the regional locale, so compare in that language.
                    if (alternative.label === t('Cerca in tutti gli account', 'Search all accounts')) onallaccounts();
                  }}
                >
                  {alternative.label}
                </Button>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
