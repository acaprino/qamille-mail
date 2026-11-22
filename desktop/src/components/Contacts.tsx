import { AnimatePresence, motion, useIsPresent, useReducedMotion } from '../ui/motion';
import { useEffect, useRef, type ChangeEvent, type ReactNode } from 'react';
import Button from '../ui/Button';
import Checkbox from '../ui/Checkbox';
import Count from '../ui/Count';
import Icon from '../ui/Icon';
import Menu from '../ui/Menu';
import Modal from '../ui/Modal';
import Select from '../ui/Select';
import { call, desktop, errorText, initials } from '../lib/api';
import { bookLabel, folderLabel, isItalian, tr } from '../lib/i18n';
import { useReactive } from '../lib/store';
import { cn } from '../lib/utils';
import {
  emptyContact,
  type BookView,
  type ContactFields,
  type ContactPage,
  type ContactSettings,
  type ContactSyncReport,
  type ContactView,
  type DuplicateView,
  type MergeOutcome,
  type MergePreview,
} from '../lib/types';
import ContactForm from './contacts/ContactForm';
import DuplicatesPanel from './contacts/DuplicatesPanel';
import MergeDialog from './contacts/MergeDialog';
import { Input } from '../ui/Input';

export type ContactsProps = { notify: (message: string, error?: boolean) => void };

type FormState = {
  mode: 'create' | 'edit';
  initial: ContactFields;
  book: string;
  id?: string;
  targets: BookView[];
};

const spring = { type: 'spring', stiffness: 400, damping: 32 } as const;
// A plain copy (store values are proxies, which structuredClone refuses).
const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

// A row of the list that slides in when it appears and fades out when it goes. While it leaves it is
// out of the accessibility tree and inert, so the list reads as its new content at once.
function ContactListItem({ children }: { children: ReactNode }) {
  const present = useIsPresent();
  const still = useReducedMotion();
  return (
    <motion.li
      aria-hidden={present ? undefined : true}
      inert={!present}
      initial={still ? false : { opacity: 0, y: -6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={still ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, y: -6 }}
      transition={spring}
    >
      {children}
    </motion.li>
  );
}

export default function Contacts({ notify }: ContactsProps) {
  // Read after awaits, in the listener and in timers: a deep store, always current.
  const s = useReactive(() => ({
    books: [] as BookView[],
    items: [] as ContactView[],
    total: 0,
    text: '',
    // 'all' is the "All address books" choice; the Select cannot hold an empty value as a choice.
    bookFilter: 'all',
    showHidden: false,
    selectedId: null as string | null,
    duplicates: [] as DuplicateView[],
    panel: false,
    lastBook: '',
    busy: false,
    syncing: false,
    loaded: false,
    form: null as FormState | null,
    merging: null as DuplicateView | null,
    deleting: null as ContactView | null,
    deleteBooks: [] as string[],
    consent: [] as string[],
    undoId: null as number | null,
  }));
  const fileInput = useRef<HTMLInputElement>(null);
  const sequence = useRef(0);
  const notifyRef = useRef(notify);
  notifyRef.current = notify;
  const say = (message: string, error?: boolean) => notifyRef.current(message, error);

  const selected = s.items.find((c) => c.id === s.selectedId) ?? null;
  const writableOrigins = (c: ContactView) => c.origins.filter((o) => o.writable);

  async function load() {
    const mine = ++sequence.current;
    try {
      const [nextBooks, page, groups, settings] = await Promise.all([
        call<BookView[]>('contacts_books'),
        call<ContactPage>('contacts_list', {
          query: {
            text: s.text,
            book: s.bookFilter === 'all' ? null : s.bookFilter,
            include_hidden: s.showHidden,
            offset: 0,
            limit: 500,
          },
        }),
        call<DuplicateView[]>('contacts_duplicates', { includePossible: false }),
        call<ContactSettings>('contacts_settings'),
      ]);
      if (mine !== sequence.current) return;
      s.books = nextBooks;
      s.items = page.items;
      s.total = page.total;
      s.duplicates = groups;
      s.lastBook = settings.last_book ?? '';
      if (s.selectedId && !page.items.some((c) => c.id === s.selectedId)) s.selectedId = null;
      s.loaded = true;
    } catch (e) {
      say(errorText(e), true);
    }
  }

  async function run(work: () => Promise<void>) {
    s.busy = true;
    try {
      await work();
    } catch (e) {
      say(errorText(e), true);
    } finally {
      s.busy = false;
    }
  }

  async function sync(announce: boolean) {
    s.syncing = true;
    try {
      const report = await call<ContactSyncReport>('contacts_sync');
      s.consent = [
        ...new Set(
          report.books
            .filter((b) => b.consent_required)
            .map((b) => b.book.replace('google-other:', 'google:')),
        ),
      ];
      const failed = report.books.filter((b) => b.error && !b.consent_required);
      if (announce) {
        if (failed.length) say(failed.map((b) => `${b.book}: ${b.error}`).join('; '), true);
        else say(tr('Rubriche aggiornate', 'Address books updated'));
      }
      await load();
    } catch (e) {
      if (announce) say(errorText(e), true);
    } finally {
      s.syncing = false;
    }
  }

  useEffect(() => {
    const timer = setTimeout(() => void load(), 150);
    return () => clearTimeout(timer);
  }, [s.text, s.bookFilter, s.showHidden]);

  useEffect(() => {
    void sync(false);
    let unlisten: (() => void) | undefined;
    let gone = false;
    if (desktop)
      void import('@tauri-apps/api/event').then(async ({ listen }) => {
        const stop = await listen('contacts-changed', () => void load());
        if (gone) stop();
        else unlisten = stop;
      });
    return () => {
      gone = true;
      unlisten?.();
    };
  }, []);

  // Creating, editing and removing
  function newContact(initial: ContactFields = emptyContact()) {
    s.form = { mode: 'create', initial, book: s.lastBook, targets: [] };
  }
  async function addToContacts(c: ContactView) {
    await run(async () => {
      newContact(await call<ContactFields>('contacts_prefill', { id: c.id }));
    });
  }
  async function editContact(c: ContactView) {
    await run(async () => {
      const targets = await call<BookView[]>('contacts_update_targets', { id: c.id });
      s.form = { mode: 'edit', initial: copy(c.fields), book: '', id: c.id, targets };
    });
  }
  async function saveForm(book: string, fields: ContactFields) {
    if (!s.form) return;
    const current = s.form;
    const result =
      current.mode === 'create'
        ? await call<ContactView>('contacts_create', { book, fields })
        : await call<ContactView>('contacts_update', { id: current.id, fields });
    s.form = null;
    s.selectedId = result.id;
    await load();
  }
  function askDelete(c: ContactView) {
    s.deleting = c;
    s.deleteBooks = [];
  }
  async function confirmDelete() {
    const target = s.deleting;
    if (!target || !s.deleteBooks.length) return;
    await run(async () => {
      await call('contacts_delete', { id: target.id, books: [...s.deleteBooks] });
      s.deleting = null;
      await load();
      say(tr('Contatto eliminato', 'Contact deleted'));
    });
  }
  async function setHidden(c: ContactView, hidden: boolean) {
    await run(async () => {
      await call('contacts_hide', { id: c.id, hidden });
      await load();
    });
  }
  async function resolve(key: string, keep: 'mine' | 'theirs') {
    await run(async () => {
      await call('contacts_resolve_conflict', { key, keep });
      await load();
    });
  }

  // Duplicates
  const keysOf = (g: DuplicateView) => g.members.map((m) => m.key);
  async function same(g: DuplicateView) {
    await run(async () => {
      await call('contacts_same_person', { keys: keysOf(g) });
      await load();
    });
  }
  async function different(g: DuplicateView) {
    await run(async () => {
      await call('contacts_different_people', { keys: keysOf(g) });
      await load();
    });
  }
  const preview = (keys: string[], keepBooks: string[]) =>
    call<MergePreview>('contacts_merge_preview', { keys, keepBooks });
  const applyMerge = (keys: string[], keepBooks: string[], fields: ContactFields) =>
    call<MergeOutcome>('contacts_merge_apply', { keys, keepBooks, fields });
  async function mergeDone(outcome: MergeOutcome) {
    s.merging = null;
    s.undoId = outcome.journal_id;
    await load();
  }
  async function undoLastMerge() {
    const id = s.undoId;
    if (id === null) return;
    await run(async () => {
      const result = await call<MergeOutcome>('contacts_merge_undo', { journalId: id });
      s.undoId = null;
      await load();
      say(
        result.recreated.length
          ? tr(
              `Unione annullata. Su Google ${result.recreated.join(', ')} è stato ricreato come nuovo contatto.`,
              `Merge undone. On Google, ${result.recreated.join(', ')} came back as a new contact.`,
            )
          : tr('Unione annullata', 'Merge undone'),
      );
    });
  }
  const bookName = (id: string) => s.books.find((b) => b.id === id)?.label ?? id;
  async function allowRead(book: string) {
    await run(async () => {
      await call('contacts_authorize', { book, write: false });
      await sync(true);
    });
  }
  async function authorize(book: string) {
    await run(async () => {
      await call('contacts_authorize', { book, write: true });
      await load();
    });
  }

  // Files
  async function importFile(event: ChangeEvent<HTMLInputElement>) {
    const input = event.currentTarget;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    await run(async () => {
      const report = await call<{ added: number; skipped: number }>('contacts_import_vcf', {
        text: await file.text(),
      });
      await load();
      say(
        tr(
          `Importati ${report.added}, saltati ${report.skipped}`,
          `Imported ${report.added}, skipped ${report.skipped}`,
        ),
      );
      if (s.duplicates.length) s.panel = true;
    });
  }
  async function exportLocal() {
    await run(async () => {
      if (desktop) {
        // The Windows "Save as" dialog, owned by the window; never a WebView2 download (audit F3).
        const result = await call<
          { status: 'empty' } | { status: 'cancelled' } | { status: 'saved'; path: string; count: number }
        >('contacts_save_vcf', { book: 'local', italian: isItalian() });
        if (result.status === 'empty') say(tr('La rubrica locale è vuota', 'The local address book is empty'), true);
        if (result.status === 'saved')
          say(tr(`Esportati ${result.count} contatti in ${result.path}`, `Exported ${result.count} contacts to ${result.path}`));
        return;
      }
      // Browser preview: the sample book is downloaded by the browser.
      const vcf = await call<string>('contacts_export_vcf', { book: 'local' });
      if (!vcf) {
        say(tr('La rubrica locale è vuota', 'The local address book is empty'), true);
        return;
      }
      const url = URL.createObjectURL(new Blob([vcf], { type: 'text/vcard;charset=utf-8' }));
      const link = document.createElement('a');
      link.href = url;
      link.download = 'qamille-contacts.vcf';
      link.click();
      URL.revokeObjectURL(url);
    });
  }

  const syncLabel = (sync: ContactView['origins'][number]['sync']) =>
    sync === 'Synced'
      ? tr('Sincronizzato', 'Synced')
      : sync === 'Pending'
        ? tr('In attesa', 'Pending')
        : sync === 'Conflict'
          ? tr('Conflitto', 'Conflict')
          : `${tr('Errore', 'Error')}: ${sync.Error}`;
  const primaryLine = (c: ContactView) =>
    c.fields.emails[0]?.value || c.fields.phones[0]?.value || '';
  const displayName = (c: ContactView) => c.fields.name || primaryLine(c) || '—';
  // The row's second line: what the name line does not already say.
  const detailLine = (c: ContactView) =>
    [
      c.fields.name ? primaryLine(c) : c.fields.emails[0] ? c.fields.phones[0]?.value : '',
      c.hidden ? tr('nascosto', 'hidden') : '',
    ]
      .filter(Boolean)
      .join(' · ');

  const form = s.form;
  const merging = s.merging;
  const target = s.deleting;

  return (
    <section className="contacts-page" aria-labelledby="contacts-title">
      <h1 id="contacts-title" className="sr-only">
        {folderLabel('Contacts')}
      </h1>
      <div className="contacts-layout">
        <div className="contacts-side">
          <div className="contacts-toolbar">
            <Button variant="primary" onClick={() => newContact()}>
              <Icon name="plus" size={15} />
              {tr('Nuovo contatto', 'New contact')}
            </Button>
            <Button variant="secondary" disabled={s.syncing} onClick={() => void sync(true)}>
              {s.syncing ? tr('Aggiorno…', 'Updating…') : tr('Aggiorna ora', 'Refresh now')}
            </Button>
            {s.duplicates.length > 0 && (
              <Button
                variant="secondary"
                aria-expanded={s.panel}
                onClick={() => (s.panel = !s.panel)}
              >
                {tr('Duplicati', 'Duplicates')}{' '}
                <Count value={s.duplicates.length} className="nav-count" />
              </Button>
            )}
            {/* The file actions in a menu: the toolbar stays on one row in the side column. */}
            <div className="more-actions">
              <Menu
                label={tr('Azioni sulla rubrica', 'Address book actions')}
                tip={tr('Altre azioni', 'More actions')}
                trigger={
                  <Button variant="icon" aria-label={tr('Altre azioni sulla rubrica', 'More address book actions')}>
                    <Icon name="more" size={18} />
                  </Button>
                }
                items={[
                  {
                    value: 'import',
                    label: tr('Importa .vcf', 'Import .vcf'),
                    icon: 'upload',
                    onSelect: () => fileInput.current?.click(),
                  },
                  {
                    value: 'export',
                    label: tr('Esporta .vcf', 'Export .vcf'),
                    icon: 'download',
                    onSelect: () => void exportLocal(),
                  },
                ]}
              />
            </div>
            <input
              ref={fileInput}
              className="sr-only"
              type="file"
              accept=".vcf,text/vcard"
              tabIndex={-1}
              aria-hidden="true"
              onChange={(event) => void importFile(event)}
            />
          </div>
          <div className="contacts-filters">
            <Input
              type="search"
              aria-label={tr('Cerca nei contatti', 'Search contacts')}
              placeholder={tr('Cerca nome, email, telefono…', 'Search name, email, phone…')}
              value={s.text}
              onChange={(event) => (s.text = event.currentTarget.value)}
            />
            <Select
              aria-label={tr('Filtra per rubrica', 'Filter by address book')}
              value={s.bookFilter}
              onChange={(value) => (s.bookFilter = value)}
              items={[
                { value: 'all', label: tr('Tutte le rubriche', 'All address books') },
                ...s.books.map((b) => ({ value: b.id, label: bookLabel(b.label) })),
              ]}
            />
            <Checkbox checked={s.showHidden} onChange={(on) => (s.showHidden = on)}>
              {tr('Mostra nascosti', 'Show hidden')}
            </Checkbox>
          </div>
          {s.consent.map((book) => (
            <div className="contacts-consent" role="status" key={book}>
              <span>
                {tr(
                  `Per leggere i contatti di ${bookName(book)} serve il tuo permesso.`,
                  `Reading the contacts of ${bookName(book)} needs your permission.`,
                )}
              </span>
              <Button variant="secondary" disabled={s.busy} onClick={() => void allowRead(book)}>
                {tr('Consenti la lettura', 'Allow reading')}
              </Button>
            </div>
          ))}
          {s.undoId !== null && (
            <div className="contacts-undo" role="status">
              <span>{tr('Contatti uniti.', 'Contacts merged.')}</span>
              <Button variant="text" disabled={s.busy} onClick={() => void undoLastMerge()}>
                {tr('Annulla unione', 'Undo merge')}
              </Button>
              <Button
                variant="icon"
                aria-label={tr('Chiudi', 'Dismiss')}
                tip={tr('Chiudi', 'Dismiss')}
                onClick={() => (s.undoId = null)}
              >
                <Icon name="close" size={14} />
              </Button>
            </div>
          )}
          {s.panel && (
            <DuplicatesPanel
              groups={s.duplicates}
              busy={s.busy}
              same={(g) => void same(g)}
              different={(g) => void different(g)}
              merge={(g) => (s.merging = g)}
              close={() => (s.panel = false)}
            />
          )}
          <ul className="contact-list" aria-label={tr('Contatti', 'Contacts')}>
            <AnimatePresence initial={false}>
              {s.items.map((c) => (
                <ContactListItem key={c.id}>
                  <button
                    className={cn(
                      'contact-row',
                      c.id === s.selectedId && 'active',
                      c.hidden && 'contact-hidden',
                    )}
                    aria-current={c.id === s.selectedId ? 'true' : undefined}
                    onClick={() => (s.selectedId = c.id)}
                  >
                    <span className="account-avatar">{initials(displayName(c))}</span>
                    <span className="contact-main">
                      <span className="contact-head">
                        <strong>{displayName(c)}</strong>
                        <span className="contact-origins">
                          {c.origins.map((origin) => (
                            <span
                              key={origin.key}
                              className={cn('origin-chip', !origin.writable && 'origin-readonly')}
                              title={bookLabel(origin.label)}
                            >
                              {bookLabel(origin.label)}
                            </span>
                          ))}
                        </span>
                      </span>
                      {detailLine(c) && <small className="muted">{detailLine(c)}</small>}
                    </span>
                  </button>
                </ContactListItem>
              ))}
            </AnimatePresence>
          </ul>
          {s.loaded && !s.items.length && (
            <p className="muted contacts-empty">{tr('Nessun contatto.', 'No contacts.')}</p>
          )}
          <p className="muted contacts-count">
            <Count value={s.total} /> {tr('contatti', 'contacts')}
          </p>
        </div>

        <article className="contact-detail" aria-live="polite">
          {selected ? (
            <>
              <header>
                <span className="account-avatar large">{initials(displayName(selected))}</span>
                <h2>{displayName(selected)}</h2>
              </header>
              <div className="contact-detail-actions">
                {writableOrigins(selected).length ? (
                  <>
                    <Button
                      variant="secondary"
                      disabled={s.busy}
                      onClick={() => void editContact(selected)}
                    >
                      {tr('Modifica', 'Edit')}
                    </Button>
                    <Button
                      variant="secondary"
                      disabled={s.busy}
                      onClick={() => askDelete(selected)}
                    >
                      {tr('Elimina…', 'Delete…')}
                    </Button>
                  </>
                ) : (
                  <Button
                    variant="primary"
                    disabled={s.busy}
                    onClick={() => void addToContacts(selected)}
                  >
                    {tr('Aggiungi ai contatti', 'Add to contacts')}
                  </Button>
                )}
                {selected.hidden ? (
                  <Button
                    variant="secondary"
                    disabled={s.busy}
                    onClick={() => void setHidden(selected, false)}
                  >
                    {tr('Mostra di nuovo', 'Show again')}
                  </Button>
                ) : (
                  <Button
                    variant="secondary"
                    disabled={s.busy}
                    onClick={() => void setHidden(selected, true)}
                  >
                    {tr('Nascondi', 'Hide')}
                  </Button>
                )}
              </div>
              <dl className="contact-facts">
                {(selected.fields.org || selected.fields.title) && (
                  <>
                    <dt>{tr('Azienda', 'Company')}</dt>
                    <dd>
                      {[selected.fields.title, selected.fields.org].filter(Boolean).join(' · ')}
                    </dd>
                  </>
                )}
                {selected.fields.emails.map((e, index) => (
                  <Fact key={`e${index}`} term={e.label || 'Email'} value={e.value} />
                ))}
                {selected.fields.phones.map((p, index) => (
                  <Fact
                    key={`p${index}`}
                    term={p.label || tr('Telefono', 'Phone')}
                    value={p.value}
                  />
                ))}
                {selected.fields.addresses.map((a, index) => (
                  <Fact
                    key={`a${index}`}
                    term={a.label || tr('Indirizzo', 'Address')}
                    value={a.value}
                  />
                ))}
                {selected.fields.birthday && (
                  <>
                    <dt>{tr('Compleanno', 'Birthday')}</dt>
                    <dd>{selected.fields.birthday}</dd>
                  </>
                )}
                {selected.fields.notes && (
                  <>
                    <dt>{tr('Note', 'Notes')}</dt>
                    <dd className="contact-notes">{selected.fields.notes}</dd>
                  </>
                )}
              </dl>
              <h3>{tr('Origini', 'Sources')}</h3>
              <ul className="contact-sources" aria-label={tr('Origini', 'Sources')}>
                {selected.origins.map((origin) => (
                  <li key={origin.key}>
                    <span className={cn('origin-chip', !origin.writable && 'origin-readonly')}>
                      {bookLabel(origin.label)}
                    </span>
                    <small className="muted">
                      {origin.writable ? syncLabel(origin.sync) : tr('Sola lettura', 'Read-only')}
                    </small>
                    {origin.sync === 'Conflict' && (
                      <>
                        <Button variant="text" onClick={() => void resolve(origin.key, 'mine')}>
                          {tr('Tieni la mia', 'Keep mine')}
                        </Button>
                        <Button variant="text" onClick={() => void resolve(origin.key, 'theirs')}>
                          {tr('Tieni quella di Google', "Keep Google's version")}
                        </Button>
                      </>
                    )}
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <p className="muted contact-detail-empty">
              {tr(
                'Scegli un contatto per vederne i dettagli.',
                'Choose a contact to see its details.',
              )}
            </p>
          )}
        </article>
      </div>

      {form && (
        <ContactForm
          mode={form.mode}
          books={s.books}
          initial={form.initial}
          initialBook={form.book}
          targets={form.targets}
          similar={(fields) => call<ContactView[]>('contacts_similar', { fields })}
          save={saveForm}
          cancel={() => (s.form = null)}
        />
      )}
      {merging && (
        <MergeDialog
          group={merging}
          books={s.books}
          preview={preview}
          apply={applyMerge}
          authorize={authorize}
          close={() => (s.merging = null)}
          done={(outcome) => void mergeDone(outcome)}
        />
      )}
      {target && (
        <Modal
          size="sm"
          className="contact-dialog"
          aria-label={tr('Elimina contatto', 'Delete contact')}
          onClose={() => (s.deleting = null)}
        >
          <h2>{tr('Elimina contatto', 'Delete contact')}</h2>
          <p className="muted">
            {tr('Scegli da quali rubriche eliminare', 'Choose which address books to delete from')}{' '}
            {tr('«', '“')}
            {displayName(target)}
            {tr('».', '”.')}
          </p>
          {writableOrigins(target).map((origin) => (
            <div className="merge-book" key={origin.key}>
              <Checkbox
                checked={s.deleteBooks.includes(origin.book)}
                onChange={(on) =>
                  (s.deleteBooks = on
                    ? [...new Set([...s.deleteBooks, origin.book])]
                    : s.deleteBooks.filter((b) => b !== origin.book))
                }
              >
                {bookLabel(origin.label)}
              </Checkbox>
            </div>
          ))}
          <p className="muted">
            {tr(
              'Le schede viste nella posta non si eliminano: si possono nascondere.',
              'Cards seen in mail cannot be deleted; you can hide them.',
            )}
          </p>
          <div className="contact-actions">
            <Button variant="secondary" onClick={() => (s.deleting = null)}>
              {tr('Annulla', 'Cancel')}
            </Button>
            <Button
              variant="primary"
              className="danger"
              disabled={s.busy || !s.deleteBooks.length}
              onClick={() => void confirmDelete()}
            >
              {tr('Elimina', 'Delete')}
            </Button>
          </div>
        </Modal>
      )}
    </section>
  );
}

function Fact({ term, value }: { term: string; value: string }) {
  return (
    <>
      <dt>{term}</dt>
      <dd>{value}</dd>
    </>
  );
}
