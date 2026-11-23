import { useEffect, useRef, useState } from 'react';
import Button from '../ui/Button';
import Icon from '../ui/Icon';
import MessageHtml from './MessageHtml';
import { call, errorText, fullDate } from '../lib/api';
import { humanSize } from '../lib/file-size';
import { fixedBackendText, tr } from '../lib/i18n';
import type { AttachedMessage, AttachedPart } from '../lib/types';
import './AttachedMessageView.css';

export type AttachedMessageViewProps = {
  emailId: number;
  attachmentId: number;
  path: number[];
  notify: (message: string, error?: boolean) => void;
  dark: boolean;
  /** Already parsed by an earlier visit to this level. */
  cached?: AttachedMessage | null;
  loaded: (message: AttachedMessage) => void;
  openPart: (part: AttachedPart) => void;
  /** Shows the same attachment as a document or text instead. */
  showAsFile: () => void;
};

export default function AttachedMessageView({
  emailId,
  attachmentId,
  path,
  notify,
  dark,
  cached = null,
  loaded,
  openPart,
  showAsFile,
}: AttachedMessageViewProps) {
  // The panel re-creates this view for every level (key).
  const [message, setMessage] = useState<AttachedMessage | null>(cached);
  const [errorMessage, setErrorMessage] = useState('');
  const [loading, setLoading] = useState(!cached);
  const shown = useRef(message);
  shown.current = message;
  const request = useRef(0);
  // A reply arriving after the panel moved on belongs to no level.
  useEffect(
    () => () => {
      request.current += 1;
    },
    [],
  );

  // The latest props, for a load that starts from an effect or a retry.
  const args = useRef({ emailId, attachmentId, path, loaded });
  args.current = { emailId, attachmentId, path, loaded };
  async function load() {
    const mine = ++request.current;
    const { emailId, attachmentId, path } = args.current;
    setLoading(true);
    setErrorMessage('');
    setMessage(null);
    shown.current = null;
    try {
      const result = await call<AttachedMessage>('read_attached_message', {
        emailId,
        attachmentId,
        ...(path.length ? { path: [...path] } : {}),
      });
      if (mine !== request.current) return;
      setMessage(result);
      args.current.loaded(result);
    } catch (error) {
      if (mine !== request.current) return;
      setErrorMessage(errorText(error));
    } finally {
      if (mine === request.current) setLoading(false);
    }
  }

  const pathKey = path.join('/');
  useEffect(() => {
    if (!shown.current) void load();
    // load reads the current props itself.
  }, [emailId, attachmentId, pathKey]);

  return (
    <div className="attached-message" aria-busy={loading}>
      {loading ? (
        <p className="attached-status" role="status">
          {tr('Apertura del messaggio allegato…', 'Opening the attached message…')}
        </p>
      ) : errorMessage ? (
        <div className="attached-alert" role="alert">
          <p>{fixedBackendText(errorMessage)}</p>
          <div className="attached-alert-actions">
            <Button variant="text" onClick={() => void load()}>
              {tr('Riprova', 'Try again')}
            </Button>
            <Button variant="text" onClick={showAsFile}>
              {tr('Apri come file', 'Open as a file')}
            </Button>
          </div>
        </div>
      ) : message ? (
        <>
          <h2 className="attached-subject">{message.subject}</h2>
          <div className="message-sender attached-sender">
            <div className="header-row">
              <span className="header-label">{tr('Da', 'From')}</span>{' '}
              <span className="header-value">
                <strong>{message.from_name || message.from_addr || tr('Mittente sconosciuto', 'Unknown sender')}</strong>
                {message.from_addr && message.from_addr !== message.from_name && (
                  <span className="sender-address">&lt;{message.from_addr}&gt;</span>
                )}
              </span>
            </div>
            {!!message.to.length && (
              <div className="header-row">
                <span className="header-label">{tr('A', 'To')}</span>
                <span className="header-value">{message.to.join(', ')}</span>
              </div>
            )}
            {!!message.cc.length && (
              <div className="header-row">
                <span className="header-label">Cc</span>
                <span className="header-value">{message.cc.join(', ')}</span>
              </div>
            )}
            <div className="header-row">
              <span className="header-label">{tr('Data', 'Date')}</span>{' '}
              <time className="header-value">{fullDate(message.date ?? 0, message.date == null)}</time>
            </div>
          </div>
          {message.body_html ? (
            <MessageHtml html={message.body_html} notify={notify} dark={dark} />
          ) : (
            <article className="message-body">{message.body}</article>
          )}
          {!!message.attachments.length && (
            <div className="attachments">
              <h3>
                {message.attachments.length}{' '}
                {message.attachments.length === 1 ? tr('allegato', 'attachment') : tr('allegati', 'attachments')}
              </h3>
              <div>
                {message.attachments.map((part) => (
                  <button key={part.index} className="attachment" onClick={() => openPart(part)}>
                    <Icon name={part.message ? 'mail' : 'clip'} size={16} />
                    <strong>{part.name}</strong>
                    <small>{humanSize(part.size)}</small>
                    <span className="visually-hidden">
                      {' · '}
                      {part.message ? tr('Apri messaggio', 'Open message') : tr('Anteprima', 'Preview')}
                    </span>
                  </button>
                ))}
              </div>
            </div>
          )}
        </>
      ) : null}
    </div>
  );
}
