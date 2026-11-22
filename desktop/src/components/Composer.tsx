import { motion, useIsPresent, useReducedMotion, type Variants } from '../ui/motion';
import { useEffect, useRef, useState, type DragEvent } from 'react';
import Button from '../ui/Button';
import Tooltip from '../ui/Tooltip';
import DateField from '../ui/DateField';
import Icon from '../ui/Icon';
import Select from '../ui/Select';
import { call, errorText } from '../lib/api';
import { calendarWords as dateWords } from '../lib/calendar-words';
import { copiedDraft, type ComposerHandle, type ComposerState } from '../lib/composer';
import { isItalian, tr } from '../lib/i18n';
import { carriesMail, droppedMailIds } from '../lib/mail-drag';
import { parseLocalDateTime, regionalTimeZone } from '../lib/regional';
import { useReactive } from '../lib/store';
import { tick } from '../lib/tick';
import type { Draft, UndoAction, Workspace } from '../lib/types';
import { cn } from '../lib/utils';
import RecipientInput, { splitRecipients } from './contacts/RecipientInput';
import RichEditor, { type RichEditorHandle } from './RichEditor';
import { imeComposing } from '../lib/ime';
import './Composer.css';
import { Input } from '../ui/Input';

// The status value is a stable key: the dock compares it, so only its label is translated.
export function saveStatusLabel(status: string): string {
  if (status === 'Saved to drafts') return tr('Salvato nelle bozze', status);
  if (status === 'Unsaved changes') return tr('Modifiche non salvate', status);
  if (status === 'Saving…') return tr('Salvataggio…', status);
  if (status === 'Could not save') return tr('Salvataggio non riuscito', status);
  return status;
}

export type ComposerProps = {
  initial: Draft;
  workspace: Workspace;
  close: () => void;
  changed: () => void;
  notify: (message: string, error?: boolean) => void;
  queued: (action: UndoAction) => void;
  active: boolean;
  windowClosing: boolean;
  minimize: () => void;
  register: (handle: ComposerHandle) => () => void;
  stateChanged: (state: ComposerState) => void;
  copied: (draft: Draft) => void;
};

type Imported = { paths: string[]; lease_id: string | null };

// A plain deep copy: store values are proxies, which structuredClone (and the desktop bridge) refuse.
const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
const plainHtml = (text: string) =>
  `<p>${text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\n/g, '<br>')}</p>`;
const withPending = (list: string | undefined, typed: string) =>
  [(list ?? '').trim(), ...splitRecipients(typed)].filter(Boolean).join(', ');

// The window opens by fading in where it stands (a slide would move its frame out of the place the
// layout gives it while it settles) and, minimized or closed, sinks towards the dock as it fades.
const spring = { type: 'spring', stiffness: 400, damping: 32 } as const;
const variants: Variants = {
  open: { opacity: 1, y: 0, transition: { opacity: spring, y: { duration: 0 } } },
  closed: { opacity: 0, y: 24, transition: spring },
};

export default function Composer(props: ComposerProps) {
  const { initial, workspace, active, windowClosing, minimize } = props;
  // The callbacks and flags as they are now, for timers, awaits and the handle given to the dock.
  const p = useRef(props);
  p.current = props;
  const still = useReducedMotion();
  const present = useIsPresent();

  const s = useReactive(() => ({
    draft: copy(initial),
    // Text typed in To, Cc or Bcc and not yet confirmed as a recipient: it counts as part of the field
    // (Send, autosave, minimize and close all see it) until it becomes a chip.
    toPending: '',
    ccPending: '',
    bccPending: '',
    editorHtml: '',
    ready: false,
    cc: !!initial.cc,
    bcc: !!initial.bcc,
    schedule: false,
    scheduleDay: '',
    scheduleTime: '',
    revision: 0,
    savedRevision: 0,
    status: 'Saved to drafts',
    sending: false,
    closing: false,
    attaching: false,
    saving: false,
    saveProblem: '',
    dropDepth: 0,
    attachingMail: 0,
  }));
  // Not drawn: plain mutable values.
  const m = useRef({
    recoveryCopy: null as Draft | null,
    timer: undefined as ReturnType<typeof setTimeout> | undefined,
    pending: null as Promise<boolean> | null,
    importLeaseIds: [] as string[],
    unmounted: false,
    lastFocus: null as HTMLElement | null,
    typedSeen: JSON.stringify(['', '', '']),
  }).current;
  const panel = useRef<HTMLDivElement>(null);
  const recipientInput = useRef<HTMLInputElement>(null);
  const editor = useRef<RichEditorHandle>(null);

  const draft = s.draft;
  const fullOf = () => ({
    to: withPending(s.draft.to, s.toPending),
    cc: withPending(s.draft.cc, s.cc ? s.ccPending : ''),
    bcc: withPending(s.draft.bcc, s.bcc ? s.bccPending : ''),
  });
  const fullTo = withPending(draft.to, s.toPending);
  // The day (DateField, 'YYYY-MM-DD') and the time (native, 'HH:mm') make the local date-time the backend reads.
  const scheduleDate = s.scheduleDay && s.scheduleTime ? `${s.scheduleDay}T${s.scheduleTime}` : '';
  const accountItems = workspace.accounts.map((account) => ({
    value: String(account.id),
    label: `${account.name} <${account.email}>`,
  }));
  const signatureItems = [
    { value: 'none', label: tr('Nessuna firma', 'No signature') },
    ...workspace.signatures.map((signature) => ({
      value: String(signature.id),
      label: signature.name,
    })),
  ];
  const dateLocale = isItalian() ? 'it-IT' : 'en-US';
  // The calendar's own words (Ark writes English); the field's placeholder and month names follow the locale.
  const calendarWords = dateWords();
  const locked = s.sending || s.closing || s.attaching || windowClosing;
  const isLocked = () => s.sending || s.closing || s.attaching || p.current.windowClosing;

  useEffect(() => {
    const unregister = p.current.register({
      flush: persist,
      close: leave,
      isBusy: () => s.sending || s.closing || s.attaching,
    });
    async function prepare() {
      try {
        s.editorHtml =
          s.draft.format === 'Html'
            ? s.draft.body
            : s.draft.format === 'Rich'
              ? await call<string>('render_body', { body: s.draft.body, format: s.draft.format })
              : plainHtml(s.draft.body);
        s.ready = true;
      } catch (e) {
        p.current.notify(errorText(e), true);
      }
    }
    void prepare();
    return () => {
      m.unmounted = true;
      unregister();
      clearTimeout(m.timer);
      void (async () => {
        if (m.pending) await m.pending;
        if (m.importLeaseIds.length) {
          await call('release_attachment_imports', { importLeaseIds: [...m.importLeaseIds] });
        }
      })().catch(() => {});
    };
  }, []);

  const dirtyNow = s.revision !== s.savedRevision;
  useEffect(() => {
    p.current.stateChanged({
      subject: draft.subject,
      recipient: fullTo,
      status: s.status,
      dirty: dirtyNow,
      busy: locked,
    });
  }, [draft.subject, fullTo, s.status, dirtyNow, locked]);

  useEffect(() => {
    if (!(active && s.ready)) return;
    void tick().then(() => {
      if (!p.current.active || isLocked()) return;
      const last = m.lastFocus;
      if (last?.isConnected && panel.current?.contains(last)) last.focus({ preventScroll: true });
      // A reply already has its recipients: it starts in the text. A new message or a forward starts in To.
      else if (s.draft.reply_to !== null && s.draft.to.trim()) editor.current?.focus();
      else recipientInput.current?.focus({ preventScroll: true });
    });
  }, [active, s.ready]);

  // Typing in a recipient box changes what is saved before there is a chip to say so.
  const typed = JSON.stringify([s.toPending, s.cc ? s.ccPending : '', s.bcc ? s.bccPending : '']);
  useEffect(() => {
    if (typed === m.typedSeen) return;
    m.typedSeen = typed;
    dirty();
  }, [typed]);

  // The draft as it is saved and sent: the typed text counts as recipients.
  function current(): Draft {
    const snapshot = copy(s.draft);
    const full = fullOf();
    snapshot.to = full.to;
    snapshot.cc = full.cc;
    snapshot.bcc = full.bcc;
    return snapshot;
  }
  // The typed text becomes part of the fields (before a send, whatever the focus did).
  function commitPending() {
    if (!s.toPending.trim() && !s.ccPending.trim() && !s.bccPending.trim()) return;
    const full = fullOf();
    s.draft.to = full.to;
    s.draft.cc = full.cc;
    s.draft.bcc = full.bcc;
    s.toPending = s.ccPending = s.bccPending = '';
    dirty();
  }
  function dirty() {
    s.revision++;
    s.status = 'Unsaved changes';
    clearTimeout(m.timer);
    m.timer = setTimeout(() => void persist(), 1200);
  }
  async function persist(): Promise<boolean> {
    clearTimeout(m.timer);
    if (m.pending) {
      const ok = await m.pending;
      if (!ok) return false;
      return persist();
    }
    if (s.revision === s.savedRevision) return true;
    const snapshot = current();
    const captured = s.revision;
    const capturedImports = [...m.importLeaseIds];
    s.saving = true;
    s.status = 'Saving…';
    m.pending = (async () => {
      try {
        const saved = await call<Draft>('save_draft', {
          draft: snapshot,
          importLeaseIds: capturedImports,
        });
        m.importLeaseIds = m.importLeaseIds.filter((id) => !capturedImports.includes(id));
        s.draft.updated = saved.updated;
        s.savedRevision = captured;
        s.saveProblem = '';
        s.status = s.revision === captured ? 'Saved to drafts' : 'Unsaved changes';
        p.current.changed();
        return true;
      } catch (e) {
        s.status = 'Could not save';
        s.saveProblem = errorText(e);
        p.current.notify(errorText(e), true);
        return false;
      } finally {
        s.saving = false;
        m.pending = null;
      }
    })();
    const ok = await m.pending;
    return ok && s.revision !== s.savedRevision ? persist() : ok;
  }
  async function leave() {
    if (isLocked()) return;
    s.closing = true;
    try {
      if (await persist()) p.current.close();
    } finally {
      s.closing = false;
    }
  }
  async function saveCopy() {
    if (isLocked() || s.saving) return;
    s.closing = true;
    clearTimeout(m.timer);
    try {
      m.recoveryCopy ??= await call<Draft>('create_draft', {
        accountId: s.draft.account_id,
        replyTo: null,
        forward: false,
      });
      const saved = await call<Draft>('save_draft', {
        draft: copiedDraft(current(), m.recoveryCopy),
        importLeaseIds: [...m.importLeaseIds],
      });
      m.importLeaseIds = [];
      p.current.changed();
      p.current.copied(saved);
    } catch (e) {
      s.saveProblem = errorText(e);
      p.current.notify(s.saveProblem, true);
    } finally {
      s.closing = false;
    }
  }
  async function queue(
    action: 'send' | 'schedule' | 'discard',
    sendAt: number | null,
    commit: boolean,
  ) {
    s.sending = true;
    try {
      if (commit) commitPending();
      if (!(await persist())) return;
      p.current.queued(
        await call<UndoAction>('queue_draft_action', {
          id: s.draft.id,
          action,
          sendAt,
        }),
      );
      p.current.changed();
      p.current.close();
    } catch (e) {
      p.current.notify(errorText(e), true);
    } finally {
      s.sending = false;
    }
  }
  async function send() {
    if (isLocked()) return;
    await queue('send', null, true);
  }
  async function later() {
    if (isLocked()) return;
    let time: number;
    try {
      time = parseLocalDateTime(scheduleDate);
    } catch (error) {
      p.current.notify(errorText(error), true);
      return;
    }
    if (!Number.isFinite(time) || time <= Date.now() / 1000) {
      p.current.notify(
        tr('Scegli una data e un’ora future', 'Choose a future date and time'),
        true,
      );
      return;
    }
    await queue('schedule', time, true);
  }
  /** Takes staged files into the draft; a reply after the composer closed only releases them. */
  function adoptImport(imported: Imported): boolean {
    if (m.unmounted) {
      if (imported.lease_id)
        void call('release_attachment_imports', { importLeaseIds: [imported.lease_id] }).catch(
          () => {},
        );
      return false;
    }
    if (imported.lease_id) m.importLeaseIds.push(imported.lease_id);
    s.draft.attachments = [...new Set([...s.draft.attachments, ...imported.paths])];
    dirty();
    return true;
  }
  async function attach() {
    if (isLocked()) return;
    s.attaching = true;
    try {
      adoptImport(await call<Imported>('pick_attachments', { italian: isItalian() }));
    } catch (e) {
      p.current.notify(errorText(e), true);
    } finally {
      s.attaching = false;
    }
  }
  function dragEnter(event: DragEvent<HTMLDivElement>) {
    if (!carriesMail(event.nativeEvent)) return;
    event.preventDefault();
    s.dropDepth += 1;
  }
  function dragOver(event: DragEvent<HTMLDivElement>) {
    if (!carriesMail(event.nativeEvent)) return;
    // Capture phase: the rich editor must not treat dropped messages as text.
    event.preventDefault();
    event.stopPropagation();
    if (event.dataTransfer) event.dataTransfer.dropEffect = isLocked() ? 'none' : 'copy';
  }
  function dragLeave(event: DragEvent<HTMLDivElement>) {
    if (!carriesMail(event.nativeEvent)) return;
    s.dropDepth = Math.max(0, s.dropDepth - 1);
  }
  async function dropMail(event: DragEvent<HTMLDivElement>) {
    if (!carriesMail(event.nativeEvent)) return;
    event.preventDefault();
    event.stopPropagation();
    s.dropDepth = 0;
    const ids = droppedMailIds(event.nativeEvent);
    if (isLocked() || !ids.length) return;
    s.attaching = true;
    s.attachingMail = ids.length;
    try {
      const imported = await call<Imported>('attach_mail_to_draft', {
        ids,
        existing: [...s.draft.attachments],
      });
      if (!adoptImport(imported)) return;
      p.current.notify(
        imported.paths.length === 1
          ? tr('1 email allegata come .eml', '1 email attached as .eml')
          : tr(
              `${imported.paths.length} email allegate come .eml`,
              `${imported.paths.length} emails attached as .eml`,
            ),
      );
    } catch (e) {
      p.current.notify(errorText(e), true);
    } finally {
      s.attaching = false;
      s.attachingMail = 0;
    }
  }
  async function discard() {
    if (isLocked()) return;
    await queue('discard', null, false);
  }

  // Minimized, the panel stays drawn (out of the accessibility tree) until it has sunk away, then it
  // is hidden; opened again it shows at once. Without motion it hides at once.
  const [gone, setGone] = useState(!active);
  if (active && gone) setGone(false);
  const hidden = still ? !active : !active && gone;
  const leaving = !present || (!active && !hidden);
  const fileName = (path: string) => path.split(/[\\/]/).pop();

  return (
    <motion.div
      ref={panel}
      id={`compose-panel-${draft.id}`}
      className={cn('composer-panel', s.dropDepth > 0 && 'mail-drop')}
      role="tabpanel"
      tabIndex={-1}
      hidden={hidden}
      aria-hidden={leaving ? true : undefined}
      aria-labelledby={`compose-tab-${draft.id}`}
      style={leaving ? { pointerEvents: 'none' } : undefined}
      variants={variants}
      initial={still ? false : 'closed'}
      animate={still ? undefined : active ? 'open' : 'closed'}
      exit={!still && active ? 'closed' : undefined}
      onAnimationComplete={(definition) => {
        if (definition === 'closed' && !p.current.active) setGone(true);
      }}
      onFocus={(event) => {
        if (
          event.target instanceof HTMLElement &&
          event.target.matches('input,textarea,select,[contenteditable="true"]')
        )
          m.lastFocus = event.target;
      }}
      onDragEnterCapture={dragEnter}
      onDragOverCapture={dragOver}
      onDragLeaveCapture={dragLeave}
      onDropCapture={(event) => void dropMail(event)}
      onKeyDownCapture={(event) => {
        if (event.key === 'Escape' && !event.defaultPrevented && !locked && !imeComposing(event.nativeEvent)) {
          event.preventDefault();
          event.stopPropagation();
          minimize();
        }
      }}
    >
      {s.dropDepth > 0 && (
        <div className="mail-drop-hint" aria-hidden="true">
          <Icon name="clip" size={18} />
          {tr('Rilascia per allegare come .eml', 'Drop to attach as .eml')}
        </div>
      )}
      <header>
        <div>
          <span className="compose-symbol">
            <Icon name="edit" size={18} />
          </span>
          <h2>{draft.reply_to ? tr('Rispondi', 'Reply') : tr('Nuovo messaggio', 'New message')}</h2>
        </div>
        <span className="save-status" role="status">
          {saveStatusLabel(s.status)}
        </span>
        <Button
          variant="icon"
          aria-label={tr('Riduci la bozza', 'Minimize draft')}
          tip={tr('Riduci nelle schede delle bozze', 'Minimize to draft tabs')}
          disabled={locked}
          onClick={minimize}
        >
          <Icon name="minimize" size={18} />
        </Button>
        <Button
          variant="icon"
          aria-label={tr('Salva e chiudi la bozza', 'Save and close draft')}
          tip={tr('Salva e chiudi', 'Save and close')}
          disabled={locked}
          onClick={() => void leave()}
        >
          <Icon name="close" />
        </Button>
      </header>
      {s.saveProblem && (
        <div className="draft-recovery" role="alert">
          <p>{s.saveProblem}</p>
          <p>
            {tr(
              'Le tue modifiche restano in questa scheda. Puoi salvarne una copia separata; la bozza già salvata resterà invariata.',
              'Your edits remain in this tab. You can save a separate copy; the existing saved draft will stay unchanged.',
            )}
          </p>
          <Button variant="secondary" disabled={locked || s.saving} onClick={() => void saveCopy()}>
            {tr('Salva modifiche come nuova bozza', 'Save edits as a new draft')}
          </Button>
          <Button
            variant="secondary"
            disabled={locked || s.saving}
            onClick={() => {
              clearTimeout(m.timer);
              p.current.close();
            }}
          >
            {tr('Scarta solo le modifiche locali e chiudi', 'Discard only local edits and close')}
          </Button>
        </div>
      )}
      <div className="compose-fields">
        <div className="field-row">
          <span>{tr('Da', 'From')}</span>
          <Select
            aria-label={tr('Da', 'From')}
            value={String(draft.account_id)}
            items={accountItems}
            disabled={locked}
            onChange={(next) => {
              s.draft.account_id = Number(next);
              dirty();
            }}
          />
        </div>
        <div className="field-row">
          <span>{tr('A', 'To')}</span>
          <RecipientInput
            label={tr('A', 'To')}
            input={recipientInput}
            pending={s.toPending}
            onPendingChange={(text) => (s.toPending = text)}
            value={draft.to}
            onValueChange={(next) => (s.draft.to = next)}
            oninput={dirty}
            placeholder="name@example.com"
            disabled={locked}
          />
          <Button
            variant="text"
            className="cc-toggle"
            disabled={locked}
            aria-expanded={s.cc}
            onClick={() => (s.cc = !s.cc)}
          >
            Cc
          </Button>
          <Button
            variant="text"
            className="cc-toggle"
            disabled={locked}
            aria-expanded={s.bcc}
            onClick={() => (s.bcc = !s.bcc)}
          >
            {tr('Ccn', 'Bcc')}
          </Button>
        </div>
        {s.cc && (
          <div className="field-row">
            <span>Cc</span>
            <RecipientInput
              label="Cc"
              pending={s.ccPending}
              onPendingChange={(text) => (s.ccPending = text)}
              value={draft.cc}
              onValueChange={(next) => (s.draft.cc = next)}
              oninput={dirty}
              disabled={locked}
            />
          </div>
        )}
        {s.bcc && (
          <div className="field-row">
            <span>{tr('Ccn', 'Bcc')}</span>
            <RecipientInput
              label={tr('Ccn', 'Bcc')}
              pending={s.bccPending}
              onPendingChange={(text) => (s.bccPending = text)}
              value={draft.bcc}
              onValueChange={(next) => (s.draft.bcc = next)}
              oninput={dirty}
              disabled={locked}
            />
          </div>
        )}
        <label className="field-row">
          <span>{tr('Oggetto', 'Subject')}</span>
          <Input
            aria-label={tr('Oggetto', 'Subject')}
            value={draft.subject}
            onChange={(event) => {
              s.draft.subject = event.currentTarget.value;
              dirty();
            }}
            placeholder={tr('Aggiungi un oggetto', 'Add a subject')}
            disabled={locked}
          />
        </label>
      </div>
      <div className="composer-body">
        {s.ready ? (
          <RichEditor
            ref={editor}
            html={s.editorHtml}
            disabled={locked}
            onchange={(html) => {
              s.draft.body = html;
              s.draft.format = 'Html';
              dirty();
            }}
          />
        ) : (
          <p className="muted">{tr('Apertura dell’editor…', 'Opening editor…')}</p>
        )}
        {draft.quote ? (
          <details className="quoted-message">
            <summary>{tr('Messaggio citato', 'Quoted message')}</summary>
            <pre>{draft.quote}</pre>
          </details>
        ) : null}
        {s.attachingMail > 0 && (
          <p className="compose-attaching" role="status">
            {s.attachingMail === 1
              ? tr(
                  'Preparazione dell’email come allegato…',
                  'Preparing the email as an attachment…',
                )
              : tr(
                  `Preparazione di ${s.attachingMail} email come allegati…`,
                  `Preparing ${s.attachingMail} emails as attachments…`,
                )}
          </p>
        )}
        {draft.attachments.length > 0 && (
          <div className="compose-attachments">
            {draft.attachments.map((path) => (
              <span key={path}>
                <Icon name="clip" size={14} />
                {fileName(path)}
                <Tooltip text={`${tr('Rimuovi', 'Remove')} ${fileName(path)}`}>
                  <button
                    aria-label={`${tr('Rimuovi', 'Remove')} ${fileName(path)}`}
                    disabled={locked}
                    onClick={() => {
                      s.draft.attachments = s.draft.attachments.filter((item) => item !== path);
                      dirty();
                    }}
                  >
                    ×
                  </button>
                </Tooltip>
              </span>
            ))}
          </div>
        )}
      </div>
      {s.schedule && (
        <div className="schedule-row">
          <span className="schedule-label">
            {tr('Data e ora di invio', 'Send at')} ({regionalTimeZone()})
          </span>
          <DateField
            aria-label={tr('Data di invio', 'Send date')}
            locale={dateLocale}
            translations={calendarWords}
            disabled={locked}
            value={s.scheduleDay}
            onChange={(value) => (s.scheduleDay = value)}
          />
          <Input
            className="time-field"
            type="time"
            aria-label={tr('Ora di invio', 'Send time')}
            value={s.scheduleTime}
            onChange={(event) => (s.scheduleTime = event.currentTarget.value)}
            disabled={locked}
          />
          <Button variant="primary" disabled={locked} onClick={() => void later()}>
            {tr('Programma', 'Schedule')}
          </Button>
          <small>
            {tr('Per inviare, Qamille deve restare aperto.', 'Qamille must be open to send.')}
          </small>
        </div>
      )}
      <footer>
        <div className="send-group">
          <Button
            variant="primary"
            disabled={locked || !fullTo.trim() || !s.ready}
            onClick={() => void send()}
          >
            <Icon name="send" size={16} />
            {s.sending ? tr('Invio…', 'Sending…') : tr('Invia', 'Send')}
          </Button>
          {/* Beside Send, the two tools look alike and weigh less than it; scheduling says when it is on. */}
          <Button
            variant="icon"
            className={cn(s.schedule && 'active')}
            tip={tr('Programma invio', 'Schedule send')}
            aria-label={tr('Programma invio', 'Schedule send')}
            aria-pressed={s.schedule}
            disabled={locked}
            onClick={() => (s.schedule = !s.schedule)}
          >
            <Icon name="clock" size={18} />
          </Button>
          <Button
            variant="icon"
            aria-label={tr('Allega file', 'Attach files')}
            tip={tr('Allega file', 'Attach files')}
            disabled={locked}
            onClick={() => void attach()}
          >
            <Icon name="clip" size={18} />
          </Button>
        </div>
        <span className="signature-caption" aria-hidden="true">
          {tr('Firma', 'Signature')}
        </span>
        <Select
          className="signature-select"
          aria-label={tr('Firma', 'Signature')}
          value={draft.signature_id === null ? 'none' : String(draft.signature_id)}
          items={signatureItems}
          disabled={locked}
          onChange={(next) => {
            s.draft.signature_id = next === 'none' ? null : Number(next);
            dirty();
          }}
        />
        <Button
          variant="icon"
          aria-label={tr('Elimina bozza', 'Discard draft')}
          tip={tr('Elimina bozza', 'Discard draft')}
          disabled={locked || s.saving}
          onClick={() => void discard()}
        >
          <Icon name="trash" size={18} />
        </Button>
      </footer>
    </motion.div>
  );
}
