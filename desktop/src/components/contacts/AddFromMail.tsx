import { useEffect, useRef, useState } from 'react';
import ContactForm from './ContactForm';
import { call, errorText } from '../../lib/api';
import {
  emptyContact,
  type BookView,
  type ContactFields,
  type ContactSettings,
  type ContactView,
} from '../../lib/types';

export type AddFromMailProps = {
  name: string;
  address: string;
  close: () => void;
  notify: (message: string, error?: boolean) => void;
};

export default function AddFromMail({ name, address, close, notify }: AddFromMailProps) {
  const [books, setBooks] = useState<BookView[] | null>(null);
  const [lastBook, setLastBook] = useState('');
  // The form starts from the sender as it was when the dialog opened.
  const [initial] = useState<ContactFields>(() => ({
    ...emptyContact(),
    name,
    emails: [{ label: '', value: address }],
  }));
  const handlers = useRef({ close, notify });
  handlers.current = { close, notify };

  useEffect(() => {
    void (async () => {
      try {
        const [list, settings] = await Promise.all([
          call<BookView[]>('contacts_books'),
          call<ContactSettings>('contacts_settings'),
        ]);
        setLastBook(settings.last_book ?? '');
        setBooks(list);
      } catch (e) {
        handlers.current.notify(errorText(e), true);
        handlers.current.close();
      }
    })();
  }, []);

  return books ? (
    <ContactForm
      mode="create"
      books={books}
      initial={initial}
      initialBook={lastBook}
      similar={(fields) => call<ContactView[]>('contacts_similar', { fields })}
      save={async (book, fields) => {
        await call('contacts_create', { book, fields });
        close();
      }}
      cancel={close}
    />
  ) : null;
}
