import { useEffect, useRef, useState } from 'react';
import Button from '../../ds/primitives/Button';
import Checkbox from '../../ds/primitives/Checkbox';
import Modal from '../../ds/primitives/Modal';
import { errorText } from '../../lib/api';
import { bookLabel, tr } from '../../lib/i18n';
import { useReactive } from '../../lib/store';
import type {
  BookAction,
  BookView,
  ContactFields,
  DuplicateView,
  MergeOutcome,
  MergePreview,
} from '../../lib/types';
import ContactFieldsEditor from './ContactFieldsEditor';

export type MergeDialogProps = {
  group: DuplicateView;
  books: BookView[];
  preview: (keys: string[], keep: string[]) => Promise<MergePreview>;
  apply: (keys: string[], keep: string[], fields: ContactFields) => Promise<MergeOutcome>;
  authorize: (book: string) => Promise<void>;
  close: () => void;
  done: (outcome: MergeOutcome) => void;
};

const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

export default function MergeDialog({
  group,
  books,
  preview,
  apply,
  authorize,
  close,
  done,
}: MergeDialogProps) {
  const keys = group.members.map((m) => m.key);
  const choices = books.filter((b) => b.writable);
  // Read after awaits and right after a change of `keep`: a deep store, always current.
  const s = useReactive(() => ({
    keep: [] as string[],
    actions: [] as BookAction[],
    error: '',
    busy: false,
    failures: [] as string[],
    refreshing: false,
    finished: null as MergeOutcome | null,
  }));
  // The edited result: replaced as a whole on each edit (ContactFieldsEditor never mutates it).
  const [fields, setFields] = useState<ContactFields | null>(null);
  const sequence = useRef(0);
  const deletes = s.actions.filter((a) => a.action === 'Delete').length;
  const usable = (b: BookView) => b.kind === 'Local' || b.write_granted;

  // The latest props for the async work below.
  const latest = useRef({ keys, preview });
  latest.current = { keys, preview };

  useEffect(() => {
    s.keep = [...new Set(group.members.filter((m) => m.writable).map((m) => m.book))].filter(
      (id) => {
        const book = choices.find((b) => b.id === id);
        return book ? usable(book) : false;
      },
    );
    void refresh();
  }, []);

  // Only the answer to the latest choice may be shown, and the button waits for it.
  async function refresh() {
    const mine = ++sequence.current;
    s.error = '';
    if (!s.keep.length) {
      s.actions = [];
      s.refreshing = false;
      return;
    }
    s.refreshing = true;
    try {
      const plan = await latest.current.preview(latest.current.keys, [...s.keep]);
      if (mine !== sequence.current) return;
      s.actions = plan.actions;
      setFields((current) => current ?? copy(plan.result));
    } catch (e) {
      if (mine === sequence.current) s.error = errorText(e);
    } finally {
      if (mine === sequence.current) s.refreshing = false;
    }
  }
  function toggle(id: string, on: boolean) {
    s.keep = on ? [...new Set([...s.keep, id])] : s.keep.filter((k) => k !== id);
    void refresh();
  }
  function onlyInOne() {
    s.keep = s.keep.slice(0, 1).length
      ? s.keep.slice(0, 1)
      : choices
          .filter(usable)
          .slice(0, 1)
          .map((b) => b.id);
    void refresh();
  }
  function inAll() {
    s.keep = choices.filter(usable).map((b) => b.id);
    void refresh();
  }
  function describe(action: BookAction): string {
    if (action.action === 'Create') return tr('verrà creato', 'will be created');
    if (action.action === 'Update') return tr('verrà aggiornato', 'will be updated');
    if (action.action === 'Delete') return tr('verrà rimosso', 'will be removed');
    return action.action.Unavailable;
  }
  async function confirm() {
    if (!fields || !s.keep.length) return;
    s.busy = true;
    s.error = '';
    try {
      const clean = (list: { label: string; value: string }[]) =>
        list
          .filter((i) => i.value.trim())
          .map((i) => ({ label: i.label.trim(), value: i.value.trim() }));
      const result = await apply(keys, [...s.keep], {
        ...fields,
        emails: clean(fields.emails),
        phones: clean(fields.phones),
        addresses: clean(fields.addresses),
      });
      const failures = result.per_book
        .filter((o) => !o.ok)
        .map((o) => `${choices.find((b) => b.id === o.book)?.label ?? o.book}: ${o.error ?? ''}`);
      s.failures = failures;
      if (!failures.length) done(result);
      else s.finished = result;
    } catch (e) {
      s.error = errorText(e);
    } finally {
      s.busy = false;
    }
  }

  const finished = s.finished;
  return (
    <Modal
      className="contact-dialog merge-dialog"
      aria-label={tr('Unisci contatti', 'Merge contacts')}
      onClose={close}
    >
      <h2>{tr('Unisci contatti', 'Merge contacts')}</h2>
      <ul className="merge-members" aria-label={tr('Schede da unire', 'Cards to merge')}>
        {group.members.map((member) => (
          <li key={member.key}>
            <strong>{member.fields.name || '—'}</strong>
            <span className="origin-chip">{bookLabel(member.label)}</span>
            <small className="muted">{member.fields.emails.map((e) => e.value).join(', ')}</small>
          </li>
        ))}
      </ul>
      {/* Where the card is kept comes first, in view: below the long editor it was under the buttons. */}
      <fieldset className="merge-books">
        <legend>{tr('Dove tenerlo?', 'Where to keep it?')}</legend>
        <div className="merge-shortcuts">
          <Button type="button" variant="secondary" onClick={onlyInOne}>
            {tr('Solo in una', 'Only in one')}
          </Button>
          <Button type="button" variant="secondary" onClick={inAll}>
            {tr('In tutte', 'In all')}
          </Button>
        </div>
        {choices.map((book) => (
          <div className="merge-book" key={book.id}>
            <Checkbox
              checked={s.keep.includes(book.id)}
              disabled={!usable(book)}
              onChange={(on) => toggle(book.id, on)}
            >
              {bookLabel(book.label)}
            </Checkbox>
            {!usable(book) && (
              <>
                <small className="muted">
                  {tr('permesso di scrittura non concesso', 'write permission not granted')}
                </small>
                <Button type="button" variant="text" onClick={() => void authorize(book.id)}>
                  {tr('Consenti', 'Allow')}
                </Button>
              </>
            )}
          </div>
        ))}
      </fieldset>
      {fields && (
        <>
          <h3>{tr('Contatto risultante', 'Merged contact')}</h3>
          <ContactFieldsEditor fields={fields} onFieldsChange={setFields} />
        </>
      )}
      {s.actions.length > 0 && (
        <ul className="merge-preview" aria-label={tr('Cosa succederà', 'What will happen')}>
          {s.actions.map((action, index) => (
            <li key={index} className={action.action === 'Delete' ? 'merge-delete' : undefined}>
              {bookLabel(action.label)}: {describe(action)}
            </li>
          ))}
        </ul>
      )}
      {s.error && (
        <p role="alert" className="contact-error">
          {s.error}
        </p>
      )}
      {s.failures.length > 0 && (
        <p role="alert" className="contact-error">
          {s.failures.join('; ')}
        </p>
      )}
      <div className="contact-actions">
        {finished ? (
          <>
            <p className="muted">
              {tr(
                'Le operazioni non riuscite restano in coda e verranno ritentate.',
                'Operations that failed stay queued and will be retried.',
              )}
            </p>
            <Button type="button" variant="primary" onClick={() => done(finished)}>
              {tr('Chiudi', 'Close')}
            </Button>
          </>
        ) : (
          <>
            <Button type="button" variant="secondary" onClick={close}>
              {tr('Annulla', 'Cancel')}
            </Button>
            <Button
              type="button"
              variant="primary"
              disabled={s.busy || s.refreshing || !s.keep.length || !fields}
              onClick={() => void confirm()}
            >
              {deletes
                ? tr(`Unisci ed elimina ${deletes}`, `Merge and delete ${deletes}`)
                : tr('Unisci', 'Merge')}
            </Button>
          </>
        )}
      </div>
    </Modal>
  );
}
