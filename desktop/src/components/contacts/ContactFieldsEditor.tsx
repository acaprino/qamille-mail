import { useEffect } from 'react';
import Button from '../../ds/primitives/Button';
import Field from '../../ds/primitives/Field';
import Icon from '../../ds/primitives/Icon';
import { tr } from '../../lib/i18n';
import type { ContactFields, Labeled } from '../../lib/types';

export type ContactFieldsEditorProps = {
  fields: ContactFields;
  /** Told the whole new value on every edit (the editor never changes `fields` in place). */
  onFieldsChange?: (fields: ContactFields) => void;
};

type ListKey = 'emails' | 'phones' | 'addresses';
const blank = (): Labeled => ({ label: '', value: '' });

export default function ContactFieldsEditor({ fields, onFieldsChange }: ContactFieldsEditorProps) {
  const change = (patch: Partial<ContactFields>) => onFieldsChange?.({ ...fields, ...patch });

  // Every list keeps one empty row to type into; empty rows are dropped when saving.
  useEffect(() => {
    const lists: ListKey[] = ['emails', 'phones', 'addresses'];
    if (!lists.some((key) => !fields[key].length)) return;
    const patch: Partial<ContactFields> = {};
    for (const key of lists) if (!fields[key].length) patch[key] = [blank()];
    change(patch);
    // Once, on mount.
  }, []);

  const add = (key: ListKey) => change({ [key]: [...fields[key], blank()] });
  const remove = (key: ListKey, index: number) => {
    const next = fields[key].filter((_, i) => i !== index);
    change({ [key]: next.length ? next : [blank()] });
  };
  const edit = (key: ListKey, index: number, patch: Partial<Labeled>) =>
    change({ [key]: fields[key].map((item, i) => (i === index ? { ...item, ...patch } : item)) });

  const group = (title: string, single: string, key: ListKey, addLabel: string) => (
    <fieldset className="contact-group">
      <legend>{title}</legend>
      {fields[key].map((item, index) => (
        <div className="contact-item" key={index}>
          <input
            className="contact-item-label"
            aria-label={`${single} ${index + 1} ${tr('etichetta', 'label')}`}
            placeholder={tr('lavoro, casa…', 'work, home…')}
            value={item.label}
            onChange={(event) => edit(key, index, { label: event.currentTarget.value })}
          />
          <input
            aria-label={`${single} ${index + 1}`}
            value={item.value}
            onChange={(event) => edit(key, index, { value: event.currentTarget.value })}
            autoComplete="off"
          />
          <Button
            type="button"
            variant="icon"
            aria-label={`${tr('Rimuovi', 'Remove')} ${single.toLowerCase()} ${index + 1}`}
            tip={`${tr('Rimuovi', 'Remove')} ${single.toLowerCase()}`}
            onClick={() => remove(key, index)}
          >
            <Icon name="close" size={14} />
          </Button>
        </div>
      ))}
      <Button type="button" variant="text" onClick={() => add(key)}>
        <Icon name="plus" size={13} /> {addLabel}
      </Button>
    </fieldset>
  );

  return (
    <div className="contact-fields">
      <Field>
        {tr('Nome', 'Name')}
        <input
          value={fields.name}
          onChange={(event) => change({ name: event.currentTarget.value })}
          autoComplete="off"
        />
      </Field>
      <div className="contact-row-pair">
        <Field>
          {tr('Azienda', 'Company')}
          <input
            value={fields.org}
            onChange={(event) => change({ org: event.currentTarget.value })}
            autoComplete="off"
          />
        </Field>
        <Field>
          {tr('Ruolo', 'Job title')}
          <input
            value={fields.title}
            onChange={(event) => change({ title: event.currentTarget.value })}
            autoComplete="off"
          />
        </Field>
      </div>
      {group(tr('Email', 'Emails'), 'Email', 'emails', tr('Aggiungi email', 'Add email'))}
      {group(
        tr('Telefoni', 'Phones'),
        tr('Telefono', 'Phone'),
        'phones',
        tr('Aggiungi telefono', 'Add phone'),
      )}
      {group(
        tr('Indirizzi', 'Addresses'),
        tr('Indirizzo', 'Address'),
        'addresses',
        tr('Aggiungi indirizzo', 'Add address'),
      )}
      <Field>
        {tr('Compleanno (AAAA-MM-GG)', 'Birthday (YYYY-MM-DD)')}
        <input
          value={fields.birthday}
          onChange={(event) => change({ birthday: event.currentTarget.value })}
          placeholder="1990-04-23"
        />
      </Field>
      <Field>
        {tr('Note', 'Notes')}
        <textarea
          rows={3}
          value={fields.notes}
          onChange={(event) => change({ notes: event.currentTarget.value })}
        />
      </Field>
    </div>
  );
}
