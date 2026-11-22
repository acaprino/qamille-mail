import { useEffect } from 'react';
import Button from '../../ds/primitives/Button';
import Checkbox from '../../ds/primitives/Checkbox';
import Field from '../../ds/primitives/Field';
import Icon from '../../ds/primitives/Icon';
import { call, errorText } from '../../lib/api';
import { bookLabel, tr } from '../../lib/i18n';
import { formatDateTime } from '../../lib/regional';
import { useReactive } from '../../lib/store';
import type { BookView, ContactSettings, ContactSyncReport } from '../../lib/types';
import { Input } from '../../ui/Input';

export type ContactsSettingsProps = { notify: (message: string, error?: boolean) => void };

export default function ContactsSettings({ notify }: ContactsSettingsProps) {
  // Read again after awaits (labelOf after a sync): a store, always current.
  const s = useReactive(() => ({
    books: [] as BookView[],
    priority: [] as string[],
    roles: '',
    busy: false,
    saved: false,
  }));
  const writable = s.books.filter((b) => b.writable);
  const googleBooks = s.books.filter((b) => b.kind === 'Google' || b.kind === 'GoogleOther');
  const labelOf = (id: string) => bookLabel(s.books.find((b) => b.id === id)?.label ?? id);

  async function load() {
    try {
      const [list, settings] = await Promise.all([
        call<BookView[]>('contacts_books'),
        call<ContactSettings>('contacts_settings'),
      ]);
      s.books = list;
      const known = list.filter((b) => b.writable).map((b) => b.id);
      s.priority = [
        ...settings.priority.filter((id) => known.includes(id)),
        ...known.filter((id) => !settings.priority.includes(id)),
      ];
      s.roles = settings.roles.join(', ');
    } catch (e) {
      notify(errorText(e), true);
    }
  }
  useEffect(() => {
    void load();
  }, []);

  async function guard(work: () => Promise<void>) {
    s.busy = true;
    try {
      await work();
    } catch (e) {
      notify(errorText(e), true);
    } finally {
      s.busy = false;
    }
  }
  const toggle = (book: BookView, enabled: boolean) =>
    guard(async () => {
      await call('contacts_set_sync', { book: book.id, enabled });
      await load();
    });
  const allow = (book: BookView) =>
    guard(async () => {
      await call('contacts_authorize', { book: book.id, write: true });
      await load();
      notify(tr('Permesso di scrittura concesso', 'Write permission granted'));
    });
  const syncNow = () =>
    guard(async () => {
      const report = await call<ContactSyncReport>('contacts_sync');
      await load();
      const failed = report.books.filter((b) => b.error);
      if (failed.length)
        notify(failed.map((b) => `${labelOf(b.book)}: ${b.error}`).join('; '), true);
      else notify(tr('Rubriche aggiornate', 'Address books updated'));
    });
  function move(index: number, delta: number) {
    const target = index + delta;
    if (target < 0 || target >= s.priority.length) return;
    const next = [...s.priority];
    [next[index], next[target]] = [next[target], next[index]];
    s.priority = next;
    s.saved = false;
  }
  const save = () =>
    guard(async () => {
      const list = s.roles
        .split(/[,;\n]/)
        .map((r) => r.trim().toLowerCase())
        .filter(Boolean);
      await call('contacts_save_settings', { priority: [...s.priority], roles: list });
      s.saved = true;
      notify(tr('Impostazioni della rubrica salvate', 'Address book settings saved'));
    });

  return (
    <>
      <h2>{tr('Rubrica', 'Contacts')}</h2>
      <p className="muted">
        {tr(
          'La rubrica unificata riunisce le rubriche qui sotto e le persone viste nella posta. Nulla viene scritto su Google senza il tuo permesso.',
          'The unified address book combines the books below and the people seen in your mail. Nothing is written to Google without your permission.',
        )}
      </p>

      <h3>{tr('Rubriche', 'Address books')}</h3>
      <ul
        className="contacts-settings-books"
        aria-label={tr('Rubriche collegate', 'Connected address books')}
      >
        {googleBooks.length ? (
          googleBooks.map((book) => (
            <li key={book.id}>
              <Checkbox
                checked={book.sync_enabled}
                disabled={s.busy}
                onChange={(on) => void toggle(book, on)}
              >
                {bookLabel(book.label)}
              </Checkbox>
              <small className="muted">
                {book.last_sync
                  ? `${tr('Ultimo aggiornamento', 'Last updated')}: ${formatDateTime(book.last_sync)}`
                  : tr('Mai sincronizzata', 'Never synchronized')}
              </small>
              {book.kind === 'Google' &&
                (book.write_granted ? (
                  <small className="muted">{tr('Scrittura consentita', 'Write allowed')}</small>
                ) : (
                  <Button variant="text" disabled={s.busy} onClick={() => void allow(book)}>
                    {tr('Consenti la scrittura', 'Allow writing')}
                  </Button>
                ))}
            </li>
          ))
        ) : (
          <li className="muted">
            {tr(
              'Nessun account Google collegato: resta la rubrica locale.',
              'No Google account connected: only the local address book is used.',
            )}
          </li>
        )}
      </ul>
      <Button variant="secondary" disabled={s.busy} onClick={() => void syncNow()}>
        {tr('Aggiorna ora', 'Refresh now')}
      </Button>

      <h3>{tr('Ordine di priorità', 'Priority order')}</h3>
      <p className="field-help">
        {tr(
          'Se lo stesso dato (per esempio l’azienda) è diverso tra due rubriche, vince quella più in alto. Le persone viste nella posta contano per ultime.',
          'When the same detail (such as the company) differs between two books, the one higher up wins. People seen in mail always come last.',
        )}
      </p>
      <ol
        className="contacts-priority"
        aria-label={tr('Ordine delle rubriche', 'Order of address books')}
      >
        {s.priority.map((id, index) => (
          <li key={id}>
            <span>{labelOf(id)}</span>
            <Button
              variant="icon"
              disabled={index === 0}
              aria-label={`${tr('Sposta su', 'Move up')}: ${labelOf(id)}`}
              onClick={() => move(index, -1)}
            >
              <span className="chevron-up">
                <Icon name="chevron" size={16} />
              </span>
            </Button>
            <Button
              variant="icon"
              disabled={index === s.priority.length - 1}
              aria-label={`${tr('Sposta giù', 'Move down')}: ${labelOf(id)}`}
              onClick={() => move(index, 1)}
            >
              <span className="chevron-down">
                <Icon name="chevron" size={16} />
              </span>
            </Button>
          </li>
        ))}
      </ol>

      <h3>{tr('Indirizzi condivisi', 'Shared addresses')}</h3>
      <Field>
        {tr(
          'Parti iniziali da non considerare della stessa persona (separate da virgola)',
          'Address names never treated as one person (comma-separated)',
        )}
        <Input
          value={s.roles}
          placeholder={tr('info, vendite, segreteria', 'info, sales, office')}
          onChange={(event) => {
            s.roles = event.currentTarget.value;
            s.saved = false;
          }}
        />
      </Field>
      <p className="field-help">
        {tr(
          'Indirizzi come info@ o segreteria@ sono usati da più persone: non vengono mai proposti come duplicati.',
          'Addresses such as info@ or office@ are shared by several people: they are never proposed as duplicates.',
        )}
      </p>
      <Button
        variant="primary"
        disabled={s.busy || writable.length === 0}
        onClick={() => void save()}
      >
        {s.saved ? tr('Salvato', 'Saved') : tr('Salva', 'Save')}
      </Button>
    </>
  );
}
