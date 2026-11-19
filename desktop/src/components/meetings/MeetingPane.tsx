// What the right-hand side of the Meetings page shows for the meeting that is selected. It reads
// the meeting in full and what the engine can do (the detail needs both to say what is missing),
// shows "loading" or "could not read" until it has them, and hands them to MeetingDetailView.
// Every change of the open meeting (a command, a rename, a job, a new read) goes up through
// `onchange` so the list keeps its row in step. `reload` is a counter: the shell bumps it each
// time the user picks a meeting, so picking the open one again reads it again.
// The shell holds it with `ref` and hands in a meeting it read itself through `accept`.
import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react';
import Button from '../../ds/primitives/Button';
import { errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';
import { meetingCall, type CaptureSnapshot, type MeetingDetail, type MeetingReadiness } from '../../lib/meetings';
import type { CalendarEvent } from '../../lib/types';
import MeetingDetailView from './MeetingDetailView';
import './MeetingPane.css';

export type MeetingPaneHandle = {
  /**
   * The shell read the open meeting for its own reasons (the job it follows moved on) and hands it
   * in: it is shown like any other change.
   */
  accept: (next: MeetingDetail) => void;
};

export type MeetingPaneProps = {
  /** The selected meeting; `null` when nothing is selected. */
  meetingId: number | null;
  reload: number;
  tab: 'summary' | 'transcript';
  highlight: string | null;
  events: CalendarEvent[];
  live: CaptureSnapshot;
  notify: (message: string, error?: boolean) => void;
  ontab: (tab: 'summary' | 'transcript') => void;
  onchange: (detail: MeetingDetail) => void;
  ondeleted: () => void;
  onsettings: (section: 'Meetings' | 'Assistant') => void;
  ref?: Ref<MeetingPaneHandle>;
};

export default function MeetingPane({
  meetingId,
  reload,
  tab,
  highlight,
  events,
  live,
  notify,
  ontab,
  onchange,
  ondeleted,
  onsettings,
  ref,
}: MeetingPaneProps) {
  const [open, setOpenState] = useState<MeetingDetail | null>(null);
  const [readiness, setReadinessState] = useState<MeetingReadiness>({ transcription_blocker: null, summary_ready: true });
  const [status, setStatus] = useState<'idle' | 'loading' | 'failed'>('idle');
  // Read after an await and from the shell's `accept`: the state and the props as they are now.
  const openRef = useRef(open);
  const readinessRef = useRef(readiness);
  const setOpen = (value: MeetingDetail | null) => {
    openRef.current = value;
    setOpenState(value);
  };
  const setReadiness = (value: MeetingReadiness) => {
    readinessRef.current = value;
    setReadinessState(value);
  };
  const latest = useRef({ meetingId, notify, onchange });
  latest.current = { meetingId, notify, onchange };
  const ticket = useRef(0);

  const event =
    open?.meeting.event_id == null ? null : (events.find((e) => e.id === open.meeting.event_id) ?? null);

  /** The meeting that is already open stays on screen while it is read again. */
  async function load(id: number) {
    const mine = ++ticket.current;
    if (openRef.current?.meeting.id !== id) {
      setOpen(null);
      setStatus('loading');
    }
    try {
      const [full, ready] = await Promise.all([
        meetingCall<MeetingDetail>('meeting_read', { id }),
        // Without the answer the detail still opens, with what it knew.
        meetingCall<MeetingReadiness>('meeting_readiness').catch(() => readinessRef.current),
      ]);
      if (mine !== ticket.current) return;
      setReadiness(ready);
      setOpen(full);
      setStatus('idle');
      latest.current.onchange(full);
    } catch (error) {
      if (mine !== ticket.current) return;
      setOpen(null);
      setStatus('failed');
      latest.current.notify(errorText(error), true);
    }
  }
  useEffect(() => {
    if (meetingId === null) {
      ticket.current++;
      setOpen(null);
      setStatus('idle');
    } else void load(meetingId);
    // `reload` asks for the same meeting to be read again.
  }, [meetingId, reload]);

  /**
   * A meeting changed. Its row follows whichever meeting it is: an answer can come back after the
   * user went to another one. Only the meeting on screen is replaced here, and a read still on its
   * way (it started before this change) must not undo it.
   */
  function changed(next: MeetingDetail) {
    latest.current.onchange(next);
    if (next.meeting.id !== latest.current.meetingId) return;
    ticket.current++;
    setOpen(next);
  }
  useImperativeHandle(ref, () => ({ accept: changed }));

  if (open && open.meeting.id === meetingId)
    return (
      <MeetingDetailView
        detail={open}
        readiness={readiness}
        live={live}
        tab={tab}
        highlight={highlight}
        event={event}
        ontab={ontab}
        onchange={changed}
        ondeleted={ondeleted}
        onsettings={onsettings}
        notify={notify}
      />
    );
  if (meetingId !== null && status === 'failed')
    return (
      <div className="pane-note">
        <p>{tr('Non riesco a leggere questa riunione.', 'This meeting could not be read.')}</p>
        <Button
          variant="secondary"
          onClick={() => {
            if (meetingId !== null) void load(meetingId);
          }}
        >
          {tr('Riprova', 'Try again')}
        </Button>
      </div>
    );
  if (meetingId !== null) return <p className="pane-note">{tr('Carico la riunione…', 'Loading the meeting…')}</p>;
  return <p className="pane-note">{tr('Seleziona una riunione', 'Select a meeting')}</p>;
}
