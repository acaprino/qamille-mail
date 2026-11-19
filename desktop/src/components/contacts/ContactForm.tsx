import { useState, type FormEvent } from 'react';
import Button from '../../ds/primitives/Button';
import Modal from '../../ds/primitives/Modal';
import Select from '../../ds/primitives/Select';
import { errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';
import type { BookView, ContactFields, ContactView } from '../../lib/types';
import ContactFieldsEditor from './ContactFieldsEditor';

export type ContactFormProps = {
  mode: 'create' | 'edit';
  books: BookView[];
  initial: ContactFields;
  initialBook?: string;
  targets?: BookView[];
  similar: (fields: ContactFields) => Promise<ContactView[]>;
  save: (book: string, fields: ContactFields) => Promise<void>;
  cancel: () => void;
};

// A plain copy (the caller may hand a store proxy, which structuredClone refuses).
const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

export default function ContactForm({
  mode,
  books,
  initial,
  initialBook = '',
  targets = [],
  similar,
  save,
  cancel,
}: ContactFormProps) {
  // The form edits a copy: later changes of the props must not overwrite what is typed.
  const [fields, setFields] = useState<ContactFields>(() => copy(initial));
  const [book, setBook] = useState(initialBook);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [matches, setMatches] = useState<ContactView[]>([]);
  const choices = books.filter((b) => b.writable);
  const title =
    mode === 'create'
      ? tr('Nuovo contatto', 'New contact')
      : tr('Modifica contatto', 'Edit contact');

  const clean = (): ContactFields => {
    const keep = (list: { label: string; value: string }[]) =>
      list
        .filter((item) => item.value.trim())
        .map((item) => ({ label: item.label.trim(), value: item.value.trim() }));
    return {
      ...fields,
      name: fields.name.trim(),
      emails: keep(fields.emails),
      phones: keep(fields.phones),
      addresses: keep(fields.addresses),
    };
  };

  async function checkSimilar() {
    if (mode !== 'create') return;
    const probe = clean();
    if (!probe.name && !probe.emails.length) {
      setMatches([]);
      return;
    }
    try {
      setMatches(await similar(probe));
    } catch {
      setMatches([]);
    }
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError('');
    if (mode === 'create' && !book) {
      setError(tr('Scegli una rubrica.', 'Choose an address book'));
      return;
    }
    setBusy(true);
    try {
      await save(book, clean());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal className="contact-dialog" aria-label={title} onClose={cancel}>
      <form onSubmit={(event) => void submit(event)}>
        <h2>{title}</h2>
        {mode === 'create' ? (
          <div className="contact-book-field">
            <Select
              label={tr('Rubrica', 'Address book')}
              placeholder={tr('Scegli una rubrica…', 'Choose an address book…')}
              value={book}
              onChange={setBook}
              aria-required="true"
              items={choices.map((choice) => ({ value: choice.id, label: choice.label }))}
            />
          </div>
        ) : targets.length ? (
          <p className="muted contact-targets">
            {tr('Verrà aggiornato in:', 'Will be updated in:')}{' '}
            {targets.map((t) => t.label).join(', ')}
          </p>
        ) : null}
        <div
          role="group"
          aria-label={tr('Dati del contatto', 'Contact details')}
          onBlur={() => void checkSimilar()}
        >
          <ContactFieldsEditor fields={fields} onFieldsChange={setFields} />
        </div>
        {matches.length > 0 && (
          <p role="status" className="contact-similar">
            {tr('Esiste già un contatto simile:', 'A similar contact already exists:')}{' '}
            {matches
              .map(
                (m) =>
                  `${m.fields.name || m.fields.emails[0]?.value || '—'} (${m.origins.map((o) => o.label).join(', ')})`,
              )
              .join('; ')}
          </p>
        )}
        {error && (
          <p role="alert" className="contact-error">
            {error}
          </p>
        )}
        <div className="contact-actions">
          <Button type="button" variant="secondary" onClick={cancel}>
            {tr('Annulla', 'Cancel')}
          </Button>
          <Button type="submit" variant="primary" disabled={busy}>
            {tr('Salva', 'Save')}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
