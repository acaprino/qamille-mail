// The RecipientsInput section of the gallery: a small address book that answers onQuery (two
// matches for "ann"), a window keydown listener that stands for App.tsx's shortcut handler (same
// guard, shortcutIgnored), and a Modal variant that counts its own closes.
import { useEffect, useState } from 'react';
import Modal from '../../../src/ui/Modal';
import RecipientsInput from '../../../src/ui/RecipientsInput';
import { shortcutIgnored } from '../../../src/lib/shortcut-guard';

type Suggestion = { email: string; name?: string };
const book: Suggestion[] = [
  { email: 'anna@acme.test', name: 'Anna Bianchi' },
  { email: 'annibale@acme.test' },
  { email: 'marco@acme.test', name: 'Marco Neri' },
];

export default function RecipientsGallery({ props = {} }: { props?: Record<string, any> }) {
  const [value, setValue] = useState<string[]>(props.value ?? []);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [shortcuts, setShortcuts] = useState(0);
  const [closes, setCloses] = useState(0);
  const query = (text: string) => {
    const needle = text.trim().toLowerCase();
    setSuggestions(needle.length < 2 ? [] : book.filter((s) => `${s.name ?? ''} ${s.email}`.toLowerCase().includes(needle)));
  };
  useEffect(() => {
    const keyboard = (e: KeyboardEvent) => {
      if (shortcutIgnored(e.target) || e.ctrlKey || e.metaKey || e.altKey) return;
      if (['n', 'r', 'e'].includes(e.key.toLowerCase())) setShortcuts((n) => n + 1);
    };
    window.addEventListener('keydown', keyboard);
    return () => window.removeEventListener('keydown', keyboard);
  }, []);
  const body = (
    <RecipientsInput
      value={value}
      onChange={setValue}
      suggestions={suggestions}
      aria-label="To"
      onQuery={query}
      invalid={(email) => !/^[^@\s]+@[^@\s]+$/.test(email)}
    />
  );
  return (
    <section aria-label="Recipients" style={{ padding: 48, display: 'flex', flexDirection: 'column', gap: 16, width: 520 }}>
      {props.modal ? (
        <Modal aria-label="Compose" onClose={() => setCloses((n) => n + 1)}>
          {body}
        </Modal>
      ) : (
        body
      )}
      <output id="value" aria-label="Recipients value">
        {JSON.stringify(value)}
      </output>
      <output id="shortcuts" aria-label="Shortcuts">
        {shortcuts}
      </output>
      <output id="closes" aria-label="Dialog closes">
        {closes}
      </output>
    </section>
  );
}
