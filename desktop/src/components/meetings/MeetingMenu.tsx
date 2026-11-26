// The "more" button of the detail header: Rinomina and Elimina. Rename opens a small dialog with
// the name in it (Enter saves, an empty name is refused); delete asks first. Both leave the focus
// on the button when they close. A rename keeps the link to the calendar event and the search
// flag as the meeting has them, and is sent with the revision that was read, so it never
// overwrites a change made elsewhere: on a conflict the meeting is read again and the user told.
import { useRef, useState } from 'react';
import Button from '../../ui/Button';
import Field from '../../ui/Field';
import Icon from '../../ui/Icon';
import Menu from '../../ui/Menu';
import Modal, { ModalFooter } from '../../ui/Modal';
import { errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';
import { deleteMeeting, MeetingError, meetingCall, type MeetingDetail } from '../../lib/meetings';
import { tick } from '../../lib/tick';
import './MeetingMenu.css';
import { Input } from '../../ui/Input';

export type MeetingMenuProps = {
  detail: MeetingDetail;
  /** The name the page shows for the meeting (the header's, with its fallbacks). */
  title: string;
  notify: (message: string, error?: boolean) => void;
  onchange: (detail: MeetingDetail) => void;
  ondeleted: () => void;
};

export default function MeetingMenu({ detail, title, notify, onchange, ondeleted }: MeetingMenuProps) {
  const [dialog, setDialog] = useState<'rename' | 'delete' | null>(null);
  const [name, setName] = useState('');
  const [busy, setBusyState] = useState(false);
  const moreButton = useRef<HTMLButtonElement>(null);
  // Read after an await: the props and `busy` as they are now, not as the press saw them.
  const latest = useRef({ detail, notify, onchange, ondeleted });
  latest.current = { detail, notify, onchange, ondeleted };
  const busyRef = useRef(false);
  const setBusy = (value: boolean) => {
    busyRef.current = value;
    setBusyState(value);
  };

  const empty = name.trim() === '';

  function ask(kind: 'rename' | 'delete') {
    // The name the meeting has, not the words the page falls back on: an untitled one starts empty
    // (and stays refused until it is given a name), not with "Riunione senza nome" to save as a name.
    setName(detail.meeting.title);
    setDialog(kind);
  }
  /** Close the dialog and put the focus back on the "more" button. */
  async function close() {
    setDialog(null);
    await tick();
    moreButton.current?.focus();
  }

  async function rename() {
    if (busyRef.current || empty) return;
    setBusy(true);
    const { detail: read } = latest.current;
    try {
      latest.current.onchange(
        await meetingCall<MeetingDetail>('meeting_update', {
          id: read.meeting.id,
          title: name.trim(),
          eventId: read.meeting.event_id,
          includedInSearch: read.included_in_search,
          expectedRevision: read.revision,
        }),
      );
      await close();
    } catch (error) {
      if (error instanceof MeetingError && error.code === 'revision_conflict') {
        latest.current.notify(tr('La riunione è cambiata: l’ho riletta.', 'The meeting changed, so I read it again.'));
        try {
          latest.current.onchange(await meetingCall<MeetingDetail>('meeting_read', { id: read.meeting.id }));
        } catch {
          /* The page keeps what it had. */
        }
        await close();
      } else latest.current.notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (busyRef.current) return;
    setBusy(true);
    try {
      await deleteMeeting(latest.current.detail.meeting.id);
      setDialog(null);
      latest.current.ondeleted();
    } catch (error) {
      latest.current.notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  const ariaBusy = busy ? 'true' : undefined;

  return (
    <>
      <Menu
        label={tr('Azioni sulla riunione', 'Meeting actions')}
        tip={tr('Altre azioni', 'More actions')}
        trigger={
          <Button variant="icon" aria-label={tr('Altre azioni', 'More actions')} ref={moreButton}>
            <Icon name="more" size={18} />
          </Button>
        }
        items={[
          { value: 'rename', label: tr('Rinomina', 'Rename'), icon: 'edit', onSelect: () => ask('rename') },
          {
            value: 'delete',
            label: tr('Elimina', 'Delete'),
            icon: 'trash',
            destructive: true,
            onSelect: () => ask('delete'),
          },
        ]}
      />

      {dialog === 'rename' ? (
        <Modal
          key="rename"
          size="sm"
          className="meeting-menu-dialog"
          aria-label={tr('Rinomina riunione', 'Rename meeting')}
          onClose={() => void close()}
        >
          <h2>{tr('Rinomina riunione', 'Rename meeting')}</h2>
          <form
            className="menu-form"
            onSubmit={(event) => {
              event.preventDefault();
              void rename();
            }}
          >
            <Field>
              <span>{tr('Nome', 'Name')}</span>
              <Input value={name} onChange={(event) => setName(event.target.value)} maxLength={300} autoComplete="off" />
            </Field>
            <ModalFooter>
              <Button type="button" variant="secondary" onClick={() => void close()}>
                {tr('Annulla', 'Cancel')}
              </Button>
              <Button type="submit" variant="primary" disabled={empty} aria-busy={ariaBusy}>
                {tr('Salva', 'Save')}
              </Button>
            </ModalFooter>
          </form>
        </Modal>
      ) : dialog === 'delete' ? (
        <Modal
          key="delete"
          size="sm"
          className="meeting-menu-dialog"
          aria-label={tr('Elimina riunione', 'Delete meeting')}
          onClose={() => void close()}
        >
          <h2>{tr('Elimina riunione', 'Delete meeting')}</h2>
          <p className="menu-text">
            {tr(
              `Eliminare «${title}»? La registrazione e il testo vengono rimossi. Non si può annullare.`,
              `Delete “${title}”? The recording and the text are removed. This cannot be undone.`,
            )}
          </p>
          <ModalFooter>
            <Button variant="secondary" onClick={() => void close()}>
              {tr('Annulla', 'Cancel')}
            </Button>
            <Button variant="primary" className="danger" aria-busy={ariaBusy} onClick={() => void remove()}>
              {tr('Elimina', 'Delete')}
            </Button>
          </ModalFooter>
        </Modal>
      ) : null}
    </>
  );
}
