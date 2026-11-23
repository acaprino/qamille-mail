import { useState } from 'react';
import Button from '../ui/Button';
import Icon from '../ui/Icon';
import Modal from '../ui/Modal';
import '../print.css';
import { call, desktop, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { formatDateTime, fullDate } from '../lib/regional';
import type { Email } from '../lib/types';

export type PrintPreviewProps = {
  emails: Email[];
  title: string;
  close: () => void;
};

export default function PrintPreview({ emails, title, close }: PrintPreviewProps) {
  const [printing, setPrinting] = useState(false);
  const [printError, setPrintError] = useState('');
  async function print() {
    setPrinting(true);
    setPrintError('');
    try {
      if (desktop) await call('print_mail');
      else window.print();
    } catch (error) {
      setPrintError(errorText(error));
    } finally {
      setPrinting(false);
    }
  }

  return (
    <Modal
      size="lg"
      className="mail-print-preview"
      aria-label={tr('Anteprima di stampa dell’email', 'Email print preview')}
      onClose={close}
    >
      <div className="print-controls">
        <div>
          <strong>{tr('Anteprima di stampa', 'Print preview')}</strong>
          <span>
            {emails.length} {emails.length === 1 ? tr('messaggio', 'message') : tr('messaggi', 'messages')} · {title}
          </span>
          {printError && (
            <p className="print-error" role="alert">
              {printError}
            </p>
          )}
        </div>{' '}
        <Button variant="primary" onClick={print} disabled={printing}>
          <Icon name="print" size={17} />
          {printing ? tr('Apertura della finestra di stampa…', 'Opening print dialog…') : tr('Stampa / Salva PDF', 'Print / Save PDF')}
        </Button>{' '}
        <Button
          variant="icon"
          aria-label={tr('Chiudi l’anteprima di stampa', 'Close print preview')}
          tip={tr('Chiudi l’anteprima', 'Close preview')}
          onClick={close}
        >
          <Icon name="close" />
        </Button>
      </div>
      <div className="print-pages">
        {emails.map((email, index) => (
          <article key={`${email.id}:${index}`} className="print-message">
            <p className="print-counter">
              {index + 1} / {emails.length}
            </p>
            <h1>{email.subject || tr('(Senza oggetto)', '(No subject)')}</h1>
            <dl>
              <dt>{tr('Da', 'From')}</dt>
              <dd>
                {email.from_name} &lt;{email.from_addr}&gt;
              </dd>
              <dt>{tr('A', 'To')}</dt>
              <dd>{email.to.join(', ')}</dd>
              {!!email.cc.length && (
                <>
                  <dt>Cc</dt>
                  <dd>{email.cc.join(', ')}</dd>
                </>
              )}
              <dt>{tr('Data', 'Date')}</dt>
              <dd>
                {email.date_missing
                  ? fullDate(email.date, true)
                  : formatDateTime(email.date, { dateStyle: 'full', timeStyle: 'long' })}
              </dd>
            </dl>
            <div className="print-body">{email.body}</div>
            {!!email.attachments.length && (
              <div className="print-attachments">
                <strong>{tr('Allegati', 'Attachments')}</strong>
                <ul>
                  {email.attachments.map((file) => (
                    <li key={file.id}>
                      {file.name} ({Math.max(1, Math.round(file.size / 1024))} KB)
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </article>
        ))}
      </div>
    </Modal>
  );
}
