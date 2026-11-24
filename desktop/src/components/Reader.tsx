import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent } from 'react';
import Kbd from '../ui/Kbd';
import ConnectMailbox from './ConnectMailbox';
import Button from '../ui/Button';
import CopyButton from '../ui/CopyButton';
import Icon from '../ui/Icon';
import Menu, { type MenuItem } from '../ui/Menu';
import MessageHtml from './MessageHtml';
import AttachmentPanel from './AttachmentPanel';
import AddFromMail from './contacts/AddFromMail';
import RecipientLine from './RecipientLine';
import type { PersonIndex } from '../lib/people';
import { call, errorText, fullDate } from '../lib/api';
import { isMessageAttachment } from '../lib/attached-message';
import { humanSize } from '../lib/file-size';
import { categoryLabel, fixedBackendText, folderLabel, tr } from '../lib/i18n';
import { cn } from '../lib/utils';
import type { Account, Attachment, Email } from '../lib/types';

export type ReaderProps = {
  email: Email | null;
  opening: boolean;
  accounts: Account[];
  busyIds: Set<number>;
  action: (id: number, a: string, value?: string) => void;
  reply: (id: number, forward?: boolean) => void;
  replyAll?: (id: number) => void;
  /** Where this message stands in the open list ("2 of 12"), and how to move to the one before or after. */
  position?: number;
  total?: number;
  step?: (delta: number) => void;
  notify: (message: string, error?: boolean) => void;
  /** No mailbox at all: the empty pane offers to connect one (see ConnectMailbox). */
  connectMailbox?: { connected: () => void; otherWays: () => void };
  outputBusy: boolean;
  printEmail: (id: number, thread: boolean) => void;
  exportEmail: (id: number) => void;
  /** The messages of the conversation; the list shows them, here it only decides the print menu. */
  conversation: Email[];
  dark: boolean;
  /** The name known for each address, from the people already seen in saved mail. */
  people?: PersonIndex;
  /**
   * Whether the open view holds this message. One that is only shown (a reply of the user's own
   * in Sent, under an Inbox conversation) can be read and answered, not managed from here.
   */
  inView?: boolean;
};

const nothing = () => {};
const noPeople: PersonIndex = new Map();

export default function Reader({
  email,
  opening,
  accounts,
  busyIds,
  action,
  reply,
  replyAll = nothing,
  position = 0,
  total = 0,
  step = nothing,
  notify,
  outputBusy,
  printEmail,
  exportEmail,
  conversation,
  dark,
  people = noPeople,
  inView = true,
  connectMailbox,
}: ReaderProps) {
  // The reader's own addresses: on a message they are one person, named "you".
  const self = new Set(accounts.map((account) => account.email.trim().toLowerCase()));
  // The message on screen now, for answers that arrive after it may have changed.
  const current = useRef(email);
  current.current = email;

  // Only this message's own actions wait for the backend (INV-006).
  const busy = !!email && busyIds.has(email.id);
  // The actions that change a message wait for the backend, and are not offered at all on a message
  // the open view does not hold (INV-002): they would act on mail in another folder.
  const locked = busy || !inView;
  const whyLocked = inView
    ? ''
    : email?.folder === 'Sent'
      ? tr('Messaggio inviato: si gestisce da Inviati', 'Sent message: manage it from Sent')
      : tr(
          'Questo messaggio non è in questa vista: si gestisce dalla sua cartella',
          'This message is not in this view: manage it from its folder',
        );
  // Whether the sender is already in an address book; if not, the reader offers to add them.
  const [senderKnown, setSenderKnown] = useState(true);
  const [addingSender, setAddingSender] = useState(false);
  const senderCheck = useRef(0);
  async function checkSender() {
    const address = current.current?.from_addr?.trim().toLowerCase();
    const mine = ++senderCheck.current;
    if (!address) {
      setSenderKnown(true);
      return;
    }
    try {
      const page = await call<{
        items: { fields: { emails: { value: string }[] }; origins: { writable: boolean }[] }[];
      }>('contacts_list', {
        query: { text: address, book: null, include_hidden: true, offset: 0, limit: 20 },
      });
      if (mine !== senderCheck.current) return;
      setSenderKnown(
        page.items.some(
          (c) =>
            c.origins.some((o) => o.writable) && c.fields.emails.some((e) => e.value.trim().toLowerCase() === address),
        ),
      );
    } catch {
      if (mine === senderCheck.current) setSenderKnown(true);
    }
  }
  useEffect(() => {
    void checkSender();
    // checkSender reads the message through `current`.
  }, [email?.from_addr]);
  const [summary, setSummary] = useState('');
  const [summarizing, setSummarizing] = useState(false);
  const [previewAttachment, setPreviewAttachment] = useState<Attachment | null>(null);
  const [menu, setMenu] = useState(false);
  // The reading area outlives the message it shows: the next one starts at the top.
  const scroller = useRef<HTMLDivElement>(null);
  // Another message drops what belonged to the last one before anything is drawn for it.
  const [shownId, setShownId] = useState(email?.id);
  if (shownId !== email?.id) {
    setShownId(email?.id);
    setSummary('');
    // The request still running belongs to the message just left: this one is not waiting for it.
    setSummarizing(false);
    setPreviewAttachment(null);
    setMenu(false);
  }
  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = 0;
  }, [email?.id]);
  // The commands behind "More actions": the message's output, then what can change for it. Only what
  // can change is offered: nothing to move back from the Inbox, no snooze to remove on a message that
  // is not snoozed. Set aside is a switch and shows its state.
  function messageCommands(shown: Email): MenuItem[] {
    const commands: MenuItem[] = [
      {
        value: 'print',
        label: tr('Stampa email', 'Print email'),
        icon: 'print',
        disabled: outputBusy,
        onSelect: () => printEmail(shown.id, false),
      },
    ];
    if (conversation.length !== 1)
      commands.push({
        value: 'print-thread',
        label: tr('Stampa conversazione', 'Print thread'),
        icon: 'print',
        disabled: outputBusy,
        onSelect: () => printEmail(shown.id, true),
      });
    commands.push({
      value: 'eml',
      label: tr('Scarica .eml', 'Download .eml'),
      icon: 'download',
      disabled: outputBusy,
      onSelect: () => exportEmail(shown.id),
    });
    const state = { group: 'state', disabled: !inView, title: whyLocked || undefined };
    if (shown.folder !== 'Inbox')
      commands.push({
        ...state,
        value: 'inbox',
        label: tr('Sposta in Posta in arrivo', 'Move to inbox'),
        onSelect: () => action(shown.id, 'moveToInbox'),
      });
    if (shown.snoozed_until != null)
      commands.push({
        ...state,
        value: 'unsnooze',
        label: tr('Annulla posticipo', 'Remove snooze'),
        onSelect: () => action(shown.id, 'unsnooze'),
      });
    commands.push({
      ...state,
      value: 'set-aside',
      label: tr('Metti da parte', 'Set aside'),
      checked: shown.set_aside,
      onSelect: () => action(shown.id, 'setAside'),
    });
    return commands;
  }
  async function summarize() {
    if (!email) return;
    const id = email.id;
    setSummarizing(true);
    try {
      const text = await call<string>('summarize_mail', { id });
      if (current.current?.id === id) setSummary(text);
    } catch (e) {
      notify(errorText(e), true);
    } finally {
      // A late answer for another message must not end the wait of the one on screen.
      if (current.current?.id === id) setSummarizing(false);
    }
  }
  const attachmentOpener = useRef<HTMLElement | null>(null);
  function toggleAttachment(file: Attachment, event: MouseEvent) {
    attachmentOpener.current = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
    setPreviewAttachment((open) => (open?.id === file.id ? null : file));
  }
  function closeAttachment() {
    setPreviewAttachment(null);
    attachmentOpener.current?.focus();
    attachmentOpener.current = null;
  }

  const lockTip = (text: string) => (whyLocked ? undefined : text);

  return (
    <section
      className={cn('reader', opening && 'opening')}
      aria-label={tr('Pannello di lettura', 'Reading pane')}
      aria-busy={busy || opening}
    >
      {opening && !email ? (
        // Nothing to keep on screen yet: the first message of a view.
        <div className="reader-empty" role="status">
          <div className="empty-art">
            <Icon name="mail" size={36} />
          </div>
          <h2>{tr('Apertura del messaggio…', 'Opening message…')}</h2>
          <p>{tr('Caricamento del contenuto dell’email.', 'Loading the email contents.')}</p>
        </div>
      ) : email ? (
        <>
          <div className="reader-toolbar">
            <div className="toolbar-cluster reply-cluster">
              <Button
                variant="primary"
                className="reader-reply"
                tip={tr('Rispondi', 'Reply')}
                shortcut="R"
                aria-keyshortcuts="R"
                onClick={() => reply(email.id)}
              >
                <Icon name="reply" size={16} />
                {tr('Rispondi', 'Reply')}
              </Button>
              <Button
                variant="secondary"
                className="reader-reply-all"
                aria-label={tr('Rispondi a tutti', 'Reply all')}
                tip={tr('Rispondi a tutti', 'Reply all')}
                onClick={() => replyAll(email.id)}
              >
                <Icon name="replyAll" size={16} />
                <span>{tr('Rispondi a tutti', 'Reply all')}</span>
              </Button>
              <Button
                variant="secondary"
                className="reader-forward"
                aria-label={tr('Inoltra', 'Forward')}
                tip={tr('Inoltra', 'Forward')}
                onClick={() => reply(email.id, true)}
              >
                <Icon name="arrow" size={16} />
                <span>{tr('Inoltra', 'Forward')}</span>
              </Button>
            </div>{' '}
            <span className="toolbar-divider"></span>{' '}
            <div className="toolbar-cluster">
              <Button
                variant="icon"
                tip={lockTip(tr('Archivia', 'Archive'))}
                shortcut="E"
                aria-keyshortcuts="E"
                title={whyLocked || undefined}
                aria-label={tr('Archivia messaggio', 'Archive message')}
                disabled={locked}
                onClick={() => action(email.id, 'archive')}
              >
                <Icon name="archive" size={18} />
              </Button>
              <Button
                variant="icon"
                tip={lockTip(tr('Sposta nel cestino', 'Move to trash'))}
                title={whyLocked || undefined}
                aria-label={tr('Sposta il messaggio nel cestino', 'Move message to trash')}
                disabled={locked}
                onClick={() => action(email.id, 'moveToTrash')}
              >
                <Icon name="trash" size={18} />
              </Button>
              <Button
                variant="icon"
                tip={lockTip(tr('Posticipa a domani', 'Snooze until tomorrow'))}
                title={whyLocked || undefined}
                aria-label={tr('Posticipa a domani', 'Snooze until tomorrow')}
                disabled={locked}
                onClick={() => action(email.id, 'snooze', 'Tomorrow')}
              >
                <Icon name="clock" size={18} />
              </Button>
              <Button
                variant="icon"
                className="done-button"
                tip={lockTip(tr('Fatto', 'Done'))}
                title={whyLocked || undefined}
                aria-label={tr('Fatto', 'Done')}
                disabled={locked}
                onClick={() => action(email.id, 'markAsDone')}
              >
                <Icon name="check" size={18} />
              </Button>
              <span className="toolbar-divider"></span>
              <Button
                variant="icon"
                className={cn(email.pinned && 'accent')}
                tip={lockTip(email.pinned ? tr('Rimuovi fissaggio', 'Unpin') : tr('Fissa', 'Pin'))}
                title={whyLocked || undefined}
                aria-label={
                  email.pinned ? tr('Rimuovi fissaggio dal messaggio', 'Unpin message') : tr('Fissa messaggio', 'Pin message')
                }
                disabled={locked}
                onClick={() => action(email.id, email.pinned ? 'unpin' : 'pin')}
              >
                <Icon name="pin" size={18} />
              </Button>
              <Button
                variant="icon"
                tip={lockTip(tr('Segna come non letto', 'Mark unread'))}
                title={whyLocked || undefined}
                aria-label={tr('Segna il messaggio come non letto', 'Mark message unread')}
                disabled={locked}
                onClick={() => action(email.id, 'markAsUnseen')}
              >
                <Icon name="mail" size={18} />
              </Button>{' '}
              <div className="more-actions">
                <Menu
                  open={menu}
                  onOpenChange={setMenu}
                  label={tr('Azioni sul messaggio', 'Message actions')}
                  tip={tr('Altre azioni', 'More actions')}
                  trigger={
                    <Button variant="icon" aria-label={tr('Altre azioni sul messaggio', 'More message actions')}>
                      <Icon name="more" size={18} />
                    </Button>
                  }
                  items={messageCommands(email)}
                />
              </div>
            </div>{' '}
            {total > 0 && position > 0 && (
              <div className="toolbar-cluster reader-steps">
                <span className="reader-position" aria-live="polite">
                  {tr(`${position} di ${total}`, `${position} of ${total}`)}
                </span>{' '}
                <Button
                  variant="icon"
                  tip={tr('Messaggio precedente', 'Previous message')}
                  shortcut="↑"
                  aria-keyshortcuts="ArrowUp"
                  aria-label={tr('Messaggio precedente', 'Previous message')}
                  disabled={position <= 1}
                  onClick={() => step(-1)}
                >
                  <span className="chevron-up">
                    <Icon name="chevron" size={16} />
                  </span>
                </Button>
                <Button
                  variant="icon"
                  tip={tr('Messaggio successivo', 'Next message')}
                  shortcut="↓"
                  aria-keyshortcuts="ArrowDown"
                  aria-label={tr('Messaggio successivo', 'Next message')}
                  disabled={position >= total}
                  onClick={() => step(1)}
                >
                  <span className="chevron-down">
                    <Icon name="chevron" size={16} />
                  </span>
                </Button>
              </div>
            )}
          </div>
          <div className="reader-scroll" ref={scroller}>
            {outputBusy && (
              <p className="output-status" role="status">
                {tr('Preparazione messaggi…', 'Preparing messages…')}
              </p>
            )}
            <div className="message-location">
              <span>{accounts.find((a) => a.id === email.account_id)?.name || tr('Casella di posta', 'Mailbox')}</span>
              <Icon name="chevron" size={12} />
              <span>{folderLabel(email.folder)}</span>
              <span className={cn('message-label', email.category === 'Priority' && 'priority')}>
                {categoryLabel(email.category)}
              </span>
            </div>
            <div className="message-heading">
              <h2 className="message-title">{email.subject || tr('(Senza oggetto)', '(No subject)')}</h2>{' '}
              {!!email.subject && (
                <CopyButton className="ms-1 size-6 min-w-0 align-middle" value={email.subject} label={tr('Copia l’oggetto', 'Copy the subject')} size="icon-xs" />
              )}
              {opening && (
                // The next message is being read; this one stays until it arrives.
                <span className="opening-mark" role="status" aria-label={tr('Apertura del messaggio…', 'Opening message…')}>
                  <span className="spinning" aria-hidden="true">
                    <Icon name="refresh" size={16} />
                  </span>
                </span>
              )}
            </div>
            <div className="message-sender">
              <div className="header-row">
                <span className="header-label">{tr('Da', 'From')}</span>{' '}
                <span className="header-value">
                  <strong>{email.from_name || email.from_addr}</strong>
                  <span className="sender-address">&lt;{email.from_addr}&gt;</span>
                  <CopyButton
                    className="ms-1 size-6 min-w-0 align-middle"
                    value={email.from_addr}
                    label={`${tr('Copia', 'Copy')} ${email.from_addr}`}
                    size="icon-xs"
                  />
                  {!senderKnown && (
                    <Button variant="text" className="add-sender" onClick={() => setAddingSender(true)}>
                      {tr('Aggiungi ai contatti', 'Add to contacts')}
                    </Button>
                  )}
                </span>
              </div>{' '}
              <RecipientLine label={tr('A', 'To')} kind="to" addresses={email.to} index={people} self={self} />{' '}
              <RecipientLine label="Cc" kind="cc" addresses={email.cc} index={people} self={self} />{' '}
              <div className="header-row">
                <span className="header-label">{tr('Data', 'Date')}</span>{' '}
                <time className="header-value">{fullDate(email.date, email.date_missing)}</time>
              </div>
            </div>
            {addingSender && (
              <AddFromMail
                name={email.from_name}
                address={email.from_addr}
                notify={notify}
                close={() => {
                  setAddingSender(false);
                  void checkSender();
                }}
              />
            )}
            <div className={cn('ai-summary', !!summary && 'expanded')}>
              <button onClick={summarize} disabled={summarizing}>
                <span className="summary-spark">
                  <Icon name="spark" size={17} />
                </span>
                <span>
                  {summarizing
                    ? tr('Preparazione del riassunto…', 'Preparing summary…')
                    : summary
                      ? tr('Riassunto del messaggio', 'Message summary')
                      : tr('Vai all’essenziale', 'Get the essentials')}
                </span>
                {!summary && (
                  <>
                    <span className="summary-hint">{tr('Riassumi con l’AI', 'Summarize with AI')}</span>
                    <Icon name="chevron" size={15} />
                  </>
                )}
              </button>
              {!!summary && <p>{fixedBackendText(summary)}</p>}
            </div>
            {email.body_html ? (
              <MessageHtml html={email.body_html} notify={notify} dark={dark} />
            ) : (
              <article className="message-body">{email.body}</article>
            )}
            {!!email.attachments.length && (
              <div className="attachments">
                <h3>
                  {email.attachments.length}{' '}
                  {email.attachments.length === 1 ? tr('allegato', 'attachment') : tr('allegati', 'attachments')}
                </h3>
                <div>
                  {email.attachments.map((file) => (
                    <button
                      key={file.id}
                      className={cn('attachment', previewAttachment?.id === file.id && 'active')}
                      aria-expanded={previewAttachment?.id === file.id}
                      onClick={(event) => toggleAttachment(file, event)}
                    >
                      <Icon name={isMessageAttachment(file.name, file.mime) ? 'mail' : 'clip'} size={16} />
                      <strong>{file.name}</strong>
                      <small>{humanSize(file.size)}</small>
                      <span className="visually-hidden">
                        {' · '}
                        {isMessageAttachment(file.name, file.mime)
                          ? tr('Apri messaggio', 'Open message')
                          : tr('Anteprima', 'Preview')}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
          {previewAttachment && (
            <AttachmentPanel
              key={`${email.id}:${previewAttachment.id}`}
              emailId={email.id}
              attachment={previewAttachment}
              notify={notify}
              dark={dark}
              close={closeAttachment}
            />
          )}
          <div className="reader-footer">
            <Icon name="info" size={13} />
            <span>
              {tr('Premi', 'Press')} <Kbd>R</Kbd> {tr('per rispondere,', 'to reply,')} <Kbd>E</Kbd>{' '}
              {tr('per archiviare', 'to archive')}
            </span>
          </div>
        </>
      ) : connectMailbox && !accounts.length ? (
        <div className="reader-empty">
          <div className="empty-art">
            <Icon name="mail" size={36} />
          </div>
          <ConnectMailbox connected={connectMailbox.connected} otherWays={connectMailbox.otherWays} />
        </div>
      ) : (
        <div className="reader-empty">
          <div className="empty-art">
            <Icon name="mail" size={36} />
          </div>
          {/* An empty list has its own note beside: no message to select, no keys to move between them. */}
          {total > 0 && (
            <>
              <h2>{tr('Un po’ più di concentrazione.', 'A little more focus.')}</h2>
              <p>{tr('Seleziona un messaggio per leggerlo qui.', 'Select a message to read it here.')}</p>
              <span>
                <Kbd>↑</Kbd>
                <Kbd>↓</Kbd> {tr('Spostati tra i messaggi', 'Move between messages')}
              </span>
            </>
          )}
        </div>
      )}
    </section>
  );
}
