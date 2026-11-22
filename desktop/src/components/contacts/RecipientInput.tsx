import { useEffect, useRef, useState, type Ref } from 'react';
import RecipientsInput from '../../ds/primitives/RecipientsInput';
import { call } from '../../lib/api';
import { tr } from '../../lib/i18n';
import { validAddress } from '../../lib/mailto';
import type { ContactSuggestion } from '../../lib/types';

// "Ann <a@x.it>, b@y.it; c@z.it" to its entries: a comma or semicolon outside quotes and angle
// brackets ends one. The text of a draft is kept this way (the backend reads it as is).
export function splitRecipients(text: string): string[] {
  const entries: string[] = [];
  let current = '';
  let quoted = false;
  let angle = false;
  for (const char of text) {
    if (char === '"') quoted = !quoted;
    else if (!quoted && char === '<') angle = true;
    else if (!quoted && char === '>') angle = false;
    if ((char === ',' || char === ';') && !quoted && !angle) {
      entries.push(current);
      current = '';
    } else current += char;
  }
  entries.push(current);
  return entries.map((entry) => entry.trim()).filter(Boolean);
}

export type RecipientInputProps = {
  value?: string | undefined;
  onValueChange?: (value: string) => void;
  /** The text box, for the caller to focus. */
  input?: Ref<HTMLInputElement>;
  /** The text typed and not yet confirmed as a recipient. */
  pending?: string;
  onPendingChange?: (value: string) => void;
  label: string;
  placeholder?: string;
  disabled?: boolean;
  oninput?: () => void;
};

export default function RecipientInput({
  value,
  onValueChange,
  input,
  pending,
  onPendingChange,
  label,
  placeholder = '',
  disabled = false,
  oninput,
}: RecipientInputProps) {
  // The draft keeps one string; the field works on its entries.
  const entries = splitRecipients(value ?? '');
  const [suggestions, setSuggestions] = useState<{ email: string; name?: string }[]>([]);
  const request = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  // The contacts that match what is being typed, after a short pause and from two letters on.
  function query(text: string) {
    clearTimeout(timer.current);
    const word = text.trim();
    const mine = ++request.current;
    if (word.length < 2) {
      setSuggestions([]);
      return;
    }
    timer.current = setTimeout(async () => {
      try {
        const found = await call<ContactSuggestion[]>('contacts_suggest', { text: word, limit: 6 });
        if (mine === request.current)
          setSuggestions(found.map((item) => ({ email: item.address, name: item.name })));
      } catch {
        if (mine === request.current) setSuggestions([]);
      }
    }, 120);
  }

  const words = {
    clearTriggerLabel: tr('Rimuovi tutti i destinatari', 'Remove all recipients'),
    deleteTagTriggerLabel: (entry: string) => tr(`Rimuovi ${entry}`, `Remove ${entry}`),
    tagAdded: (entry: string) => tr(`Aggiunto ${entry}`, `Added ${entry}`),
    tagsPasted: (list: string[]) =>
      tr(`Incollati ${list.length} destinatari`, `Pasted ${list.length} recipients`),
    tagDeleted: (entry: string) => tr(`${entry} rimosso`, `${entry} removed`),
    tagSelected: (entry: string) =>
      tr(
        `${entry} selezionato. Backspace o Canc per rimuoverlo.`,
        `${entry} selected. Press Backspace or Delete to remove it.`,
      ),
    noTagsSelected: tr('Nessun destinatario selezionato', 'No recipient selected'),
    inputLabel: (count: number) =>
      tr(`${count} destinatari`, count === 1 ? '1 recipient' : `${count} recipients`),
  };
  const listWords = {
    triggerLabel: tr('Mostra i suggerimenti', 'Show suggestions'),
    clearTriggerLabel: tr('Cancella', 'Clear'),
    resultsCount: (count: number) =>
      tr(`${count} suggerimenti`, count === 1 ? '1 suggestion' : `${count} suggestions`),
    noResults: tr('Nessun suggerimento', 'No suggestions'),
    loading: tr('Ricerca in corso', 'Searching'),
  };

  return (
    <RecipientsInput
      value={entries}
      onChange={(next) => {
        onValueChange?.(next.join(', '));
        oninput?.();
      }}
      inputRef={input}
      pending={pending}
      onPendingChange={onPendingChange}
      suggestions={suggestions}
      aria-label={label}
      placeholder={placeholder}
      disabled={disabled}
      suggestionsLabel={tr('Suggerimenti dalla rubrica', 'Suggestions from your contacts')}
      translations={words}
      listTranslations={listWords}
      onQuery={query}
      invalid={(email) => !validAddress(email)}
    />
  );
}
