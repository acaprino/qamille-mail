// The filters of the search, in one form under the search field: who sent it, to whom, when, the attachment, the folder, the
// state, and how the results are ordered; and, behind «Altri filtri», the subject, the text, who is in Cc, the account, the
// label and the category. Every field is there at once; Enter or «Applica» writes them all into the query (the operators
// from:, to:, on:, between:, around:, has:, filename:, type:, in:, is:, subject:, body:, cc:, account:, label:, category:),
// and an empty field takes its operator out. Several people in «Da» or «A», separated by commas, become one OR group. The
// query stays the one source: this only reads it when it opens and writes it when it applies.
import { useEffect, useRef, useState } from 'react';
import { calendarWords as dateWords } from '../lib/calendar-words';
import { tr } from '../lib/i18n';
import {
  aroundRange,
  calendarLabel,
  canonicalSearchDate,
  filterValue,
  filterValues,
  offsetISO,
  todayISO,
  withPeople,
  withSearchFilter,
} from '../lib/search';
import Button from '../ui/Button';
import DateField from '../ui/DateField';
import Field from '../ui/Field';
import Icon from '../ui/Icon';
import { Input } from '../ui/Input';
import Select from '../ui/Select';

export type SearchFiltersProps = {
  query: string;
  locale: string;
  zone: string;
  /** The people the address fields suggest. */
  people: Person[];
  /** The accounts «Account» chooses among, by the address the operator takes. */
  accounts: { value: string; label: string }[];
  /** The labels «Etichetta» suggests. */
  labels: string[];
  sort: string;
  /** Whether the results can be ordered by closeness to a date. */
  closest: boolean;
  onapply: (query: string) => void;
  onsort: (sort: string) => void;
};

export type Person = { name: string; address: string };
const ANY = 'any';

export default function SearchFilters({ query, locale, zone, people, accounts, labels, sort, closest, onapply, onsort }: SearchFiltersProps) {
  const around = aroundRange(query, locale);
  const people0 = (operator: string) => filterValues(query, operator).join(', ');
  const [from, setFrom] = useState(() => people0('from'));
  const [to, setTo] = useState(() => people0('to'));
  const [subject, setSubject] = useState(() => filterValue(query, 'subject'));
  const [body, setBody] = useState(() => filterValue(query, 'body'));
  const [cc, setCc] = useState(() => filterValue(query, 'cc'));
  const [account, setAccount] = useState(() => filterValue(query, 'account') || ANY);
  const [label, setLabel] = useState(() => filterValue(query, 'label'));
  const [category, setCategory] = useState(() => filterValue(query, 'category') || ANY);
  const [more, setMore] = useState(() => ['subject', 'body', 'cc', 'account', 'label', 'category'].some((operator) => filterValue(query, operator)));
  const [when, setWhen] = useState(() => (around ? 'around' : filterValue(query, 'between') ? 'between' : filterValue(query, 'on') ? 'on' : ANY));
  const [day, setDay] = useState(
    () =>
      around?.day ||
      canonicalSearchDate(filterValue(query, 'on'), locale) ||
      canonicalSearchDate(filterValue(query, 'between').split('..')[0], locale) ||
      todayISO(zone),
  );
  const [end, setEnd] = useState(() => canonicalSearchDate(filterValue(query, 'between').split('..')[1], locale) || day);
  const [radius, setRadius] = useState(() => String(around?.radius ?? 6));
  const [attachment, setAttachment] = useState(() =>
    filterValue(query, 'type') ? 'type' : filterValue(query, 'filename') ? 'filename' : filterValue(query, 'has') ? 'has' : ANY,
  );
  const [attachmentValue, setAttachmentValue] = useState(() => filterValue(query, 'type') || filterValue(query, 'filename'));
  const [folder, setFolder] = useState(() => filterValue(query, 'in') || ANY);
  const [state, setState] = useState(() => filterValue(query, 'is') || ANY);
  const first = useRef<HTMLInputElement>(null);
  const t = tr;

  useEffect(() => {
    first.current?.focus();
  }, []);

  const radiusOk = Number.isInteger(Number(radius)) && Number(radius) >= 0 && Number(radius) <= 365;
  const valid = (when !== 'between' || !!end) && (when !== 'around' || radiusOk) && (attachment !== 'type' || !!attachmentValue);

  // The people as they were when the form opened: a field left alone leaves its filter (an OR group of it included) as it is.
  const fromAtOpen = useRef(from);
  const toAtOpen = useRef(to);

  function compose() {
    let result = query;
    if (from !== fromAtOpen.current) result = withPeople(result, 'from', from.split(','));
    if (to !== toAtOpen.current) result = withPeople(result, 'to', to.split(','));
    for (const op of ['on', 'between', 'around', 'after', 'before']) result = withSearchFilter(result, op, '');
    const today = todayISO(zone);
    if (when === 'today') result = withSearchFilter(result, 'on', today);
    else if (when === 'yesterday') result = withSearchFilter(result, 'on', offsetISO(today, -1));
    else if (when === 'week') result = withSearchFilter(result, 'between', `${offsetISO(today, -6)}..${today}`);
    else if (when === 'month') {
      const last = offsetISO(`${today.slice(0, 7)}-01`, -1);
      result = withSearchFilter(result, 'between', `${last.slice(0, 7)}-01..${last}`);
    } else if (when === 'on') result = withSearchFilter(result, 'on', day);
    else if (when === 'between') result = withSearchFilter(result, 'between', `${day}..${end}`);
    else if (when === 'around') result = withSearchFilter(result, 'around', `${day}/${radius}d`);
    for (const op of ['has', 'filename', 'type']) result = withSearchFilter(result, op, '');
    if (attachment === 'has') result = withSearchFilter(result, 'has', 'attachment');
    else if (attachment === 'filename') result = withSearchFilter(result, 'filename', attachmentValue);
    else if (attachment === 'type') result = withSearchFilter(result, 'type', attachmentValue);
    result = withSearchFilter(result, 'in', folder === ANY ? '' : folder);
    result = withSearchFilter(result, 'is', state === ANY ? '' : state);
    result = withSearchFilter(result, 'subject', subject);
    result = withSearchFilter(result, 'body', body);
    result = withSearchFilter(result, 'cc', cc);
    result = withSearchFilter(result, 'account', account === ANY ? '' : account);
    result = withSearchFilter(result, 'label', label);
    result = withSearchFilter(result, 'category', category === ANY ? '' : category);
    return result;
  }

  function reset() {
    setFrom('');
    setTo('');
    setWhen(ANY);
    setAttachment(ANY);
    setAttachmentValue('');
    setFolder(ANY);
    setState(ANY);
    setSubject('');
    setBody('');
    setCc('');
    setAccount(ANY);
    setLabel('');
    setCategory(ANY);
  }

  const dates = [
    { value: ANY, label: t('Qualsiasi data', 'Any date') },
    { value: 'today', label: t('Oggi', 'Today') },
    { value: 'yesterday', label: t('Ieri', 'Yesterday') },
    { value: 'week', label: t('Ultimi 7 giorni', 'Last 7 days') },
    { value: 'month', label: t('Mese scorso', 'Last month') },
    { value: 'on', label: t('Giorno preciso', 'Exact day') },
    { value: 'between', label: t('Intervallo', 'Date range') },
    { value: 'around', label: t('Intorno a una data', 'Around a date') },
  ];
  const attachments = [
    { value: ANY, label: t('Indifferente', 'Any') },
    { value: 'has', label: t('Con un allegato', 'Has an attachment') },
    { value: 'filename', label: t('Nome del file', 'File name') },
    { value: 'type', label: t('Tipo di file', 'File type') },
  ];
  const types = ['pdf', 'docx', 'xlsx', 'csv', 'pptx', 'txt', 'jpg', 'png'].map((type) => ({ value: type, label: type.toUpperCase() }));
  const folders = [
    { value: ANY, label: t('Qualsiasi cartella', 'Any folder') },
    { value: 'inbox', label: t('Posta in arrivo', 'Inbox') },
    { value: 'sent', label: t('Inviata', 'Sent') },
    { value: 'archive', label: t('Archivio', 'Archive') },
    { value: 'spam', label: 'Spam' },
    { value: 'trash', label: t('Cestino', 'Trash') },
    { value: 'anywhere', label: t('Tutta la posta, inclusi Spam e Cestino', 'All mail, including Spam and Trash') },
  ];
  const states = [
    { value: ANY, label: t('Qualsiasi', 'Any') },
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
    { value: ANY, label: t('Qualsiasi', 'Any') },
    { value: 'personal', label: t('Personale', 'Personal') },
    { value: 'priority', label: t('Priorità', 'Priority') },
    { value: 'notification', label: t('Notifica', 'Notification') },
    { value: 'newsletter', label: 'Newsletter' },
    { value: 'invitation', label: t('Invito', 'Invitation') },
    { value: 'invitation_response', label: t('Risposta a un invito', 'Invitation response') },
  ];
  const sorts = [
    { value: 'default', label: t('Automatico', 'Automatic') },
    { value: 'date', label: t('Più recenti', 'Newest first') },
    { value: 'relevance', label: t('Più pertinenti', 'Most relevant') },
    ...(closest ? [{ value: 'closest', label: t('Più vicine alla data', 'Closest to date') }] : []),
  ];
  const words = dateWords();

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (valid) onapply(compose());
      }}
    >
      <div className="grid grid-cols-2 gap-3">
        <Field className="mb-0!">
          {t('Da', 'From')}
          <Input
            ref={first}
            value={from}
            onChange={(event) => setFrom(event.target.value)}
            list="search-people"
            autoComplete="off"
            placeholder={t('Nome o email, anche più di uno', 'Name or email, several allowed')}
          />
        </Field>
        <Field className="mb-0!">
          {t('A', 'To')}
          <Input
            value={to}
            onChange={(event) => setTo(event.target.value)}
            list="search-people"
            autoComplete="off"
            placeholder={t('Nome o email, anche più di uno', 'Name or email, several allowed')}
          />
        </Field>
      </div>
      <datalist id="search-people">
        {people.map((person, i) => (
          <option key={`${person.address}:${i}`} value={person.address}>
            {person.name} · {person.address}
          </option>
        ))}
      </datalist>
      <div className="grid grid-cols-2 gap-3">
        <Select label={t('Quando', 'When')} value={when} onChange={setWhen} items={dates} />
        <Select
          label={t('Allegato', 'Attachment')}
          value={attachment}
          onChange={(value) => {
            setAttachment(value);
            setAttachmentValue('');
          }}
          items={attachments}
        />
      </div>
      {when === 'on' || when === 'between' || when === 'around' || attachment === 'type' || attachment === 'filename' ? (
        <div className="grid grid-cols-2 items-end gap-3">
          <div className={`flex min-w-0 items-end gap-3${attachment === 'type' || attachment === 'filename' ? '' : ' col-span-2'}`}>
            {when === 'on' || when === 'between' || when === 'around' ? (
              <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                <span className="text-sm font-medium text-ink">{when === 'between' ? t('Dal giorno', 'From date') : t('Data', 'Date')}</span>
                <DateField value={day} onChange={setDay} aria-label={when === 'between' ? t('Dal giorno', 'From date') : t('Data', 'Date')} locale={locale} translations={words} />
              </div>
            ) : null}
            {when === 'between' ? (
              <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                <span className="text-sm font-medium text-ink">{t('Al giorno incluso', 'Through date')}</span>
                <DateField value={end} min={day} onChange={setEnd} aria-label={t('Al giorno incluso', 'Through date')} locale={locale} translations={words} />
              </div>
            ) : null}
            {when === 'around' ? (
              <Field className="mb-0! w-28 shrink-0">
                {t('± giorni', '± days')}
                <Input type="number" min={0} max={365} value={radius} onChange={(event) => setRadius(event.target.value)} />
              </Field>
            ) : null}
          </div>
          <div className="flex min-w-0 flex-col">
            {attachment === 'type' ? (
              <Select label={t('Tipo', 'Type')} placeholder={t('Scegli un tipo', 'Choose a type')} value={attachmentValue} onChange={setAttachmentValue} items={types} />
            ) : attachment === 'filename' ? (
              <Field className="mb-0!">
                {t('Il nome contiene', 'Name contains')}
                <Input value={attachmentValue} onChange={(event) => setAttachmentValue(event.target.value)} autoComplete="off" />
              </Field>
            ) : null}
          </div>
        </div>
      ) : null}
      {when === 'around' && radiusOk && day ? (
        <p className="text-xs text-ink-mute" data-testid="search-date-preview">
          {calendarLabel(offsetISO(day, -Number(radius)), locale)} — {calendarLabel(offsetISO(day, Number(radius)), locale)} ·{' '}
          {`${radius} ${Number(radius) === 1 ? t('giorno prima e dopo', 'day before and after') : t('giorni prima e dopo', 'days before and after')} · ${
            Number(radius) * 2 + 1
          } ${Number(radius) === 0 ? t('giorno incluso', 'day included') : t('giorni inclusi', 'days included')}`}
        </p>
      ) : null}
      <div className="grid grid-cols-3 gap-3">
        <Select label={t('Cartella', 'Folder')} value={folder} onChange={setFolder} items={folders} />
        <Select label={t('Stato', 'State')} value={state} onChange={setState} items={states} />
        <Select label={t('Ordina per', 'Order by')} value={sort || 'default'} onChange={(value) => onsort(value === 'default' ? '' : value)} items={sorts} />
      </div>
      <div className="flex flex-col gap-2 border-t border-line pt-2">
        <Button type="button" variant="text" aria-expanded={more} onClick={() => setMore(!more)} className="self-start">
          {t('Altri filtri', 'More filters')}
          <Icon name="down" size={12} />
        </Button>
        {more ? (
          <div className="grid grid-cols-2 gap-3">
            <Field className="mb-0!">
              {t('Oggetto contiene', 'Subject contains')}
              <Input value={subject} onChange={(event) => setSubject(event.target.value)} autoComplete="off" />
            </Field>
            <Field className="mb-0!">
              {t('Testo contiene', 'Body contains')}
              <Input value={body} onChange={(event) => setBody(event.target.value)} autoComplete="off" />
            </Field>
            <Field className="mb-0!">
              {t('Persona in copia', 'Person in Cc')}
              <Input value={cc} onChange={(event) => setCc(event.target.value)} list="search-people" autoComplete="off" />
            </Field>
            <Field className="mb-0!">
              {t('Etichetta', 'Label')}
              <Input value={label} onChange={(event) => setLabel(event.target.value)} list="search-labels" autoComplete="off" />
            </Field>
            <Select label="Account" value={account} onChange={setAccount} items={[{ value: ANY, label: t('Qualsiasi', 'Any') }, ...accounts]} />
            <Select label={t('Categoria', 'Category')} value={category} onChange={setCategory} items={categories} />
            <datalist id="search-labels">
              {labels.map((name) => (
                <option key={name} value={name} />
              ))}
            </datalist>
          </div>
        ) : null}
      </div>
      <div className="flex items-center justify-between gap-3 pt-1">
        <Button type="button" variant="text" onClick={reset}>
          {t('Azzera', 'Clear')}
        </Button>
        <Button type="submit" variant="primary" disabled={!valid}>
          {t('Applica', 'Apply')}
        </Button>
      </div>
    </form>
  );
}
