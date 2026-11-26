// The detail of one meeting: the head (title, when, how long, who, the menu) and, under it,
// whatever state the meeting is in (`detailState`): being recorded, being processed, to recover,
// without a transcript (and why), or ready: the Sintesi and Trascrizione tabs. It holds no data
// of its own: the shell reads the meeting and hands it in; every change goes back up through
// `onchange` with the meeting as the backend returns it, so the list stays in step.
import { useRef, useState } from 'react';
import Button from '../../ui/Button';
import Icon from '../../ui/Icon';
import Modal, { ModalFooter } from '../../ui/Modal';
import Tabs from '../../ui/Tabs';
import { errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';
import { keepFocusInDetail } from '../../lib/meetings-focus';
import {
  MeetingError,
  meetingCall,
  type CaptureSnapshot,
  type MeetingAction,
  type MeetingDetail,
  type MeetingJob,
  type MeetingReadiness,
} from '../../lib/meetings';
import { detailState, summaryPlainText, transcriptPlainText } from '../../lib/meetings-view';
import type { CalendarEvent } from '../../lib/types';
import MeetingHeader from './MeetingHeader';
import MeetingLive from './MeetingLive';
import MeetingNoTranscript from './MeetingNoTranscript';
import MeetingNotice from './MeetingNotice';
import MeetingProgress from './MeetingProgress';
import MeetingSummary from './MeetingSummary';
import MeetingTranscript from './MeetingTranscript';
import './MeetingDetailView.css';

type Tab = 'summary' | 'transcript';

export type MeetingDetailViewProps = {
  detail: MeetingDetail;
  readiness: MeetingReadiness;
  /** The recording as the capture bus has it. */
  live: CaptureSnapshot;
  tab: Tab;
  /** The transcript segment a search result pointed at. */
  highlight?: string | null;
  event?: CalendarEvent | null;
  ontab: (tab: Tab) => void;
  onchange: (detail: MeetingDetail) => void;
  ondeleted: () => void;
  /** Opens Settings on that section. */
  onsettings: (section: 'Meetings' | 'Assistant') => void;
  notify: (message: string, error?: boolean) => void;
};

export default function MeetingDetailView({
  detail,
  readiness,
  live,
  tab,
  highlight = null,
  event = null,
  ontab,
  onchange,
  ondeleted,
  onsettings,
  notify,
}: MeetingDetailViewProps) {
  const current = detailState(detail, { readiness });
  const [busy, setBusyState] = useState(false);
  const [discarding, setDiscarding] = useState(false);
  // `busy` and the props are read after an await (and twice in one event): as they are now.
  const busyRef = useRef(false);
  const setBusy = (value: boolean) => {
    busyRef.current = value;
    setBusyState(value);
  };
  const latest = useRef({ detail, onchange, notify });
  latest.current = { detail, onchange, notify };

  async function reread() {
    latest.current.onchange(await meetingCall<MeetingDetail>('meeting_read', { id: latest.current.detail.meeting.id }));
  }
  /**
   * One command on this meeting. It answers with the meeting; a refusal is shown, and the meeting
   * read again. `true`: it worked. `false`: it was sent and failed. `null`: nothing was sent,
   * because another command was still on its way.
   */
  async function run(command: string, args: Record<string, unknown> = {}): Promise<boolean | null> {
    if (busyRef.current) return null;
    setBusy(true);
    try {
      const result = await meetingCall<MeetingDetail | null>(command, { id: latest.current.detail.meeting.id, ...args });
      if (result?.meeting) latest.current.onchange(result);
      else await reread();
      return true;
    } catch (error) {
      if (error instanceof MeetingError && error.code === 'revision_conflict')
        latest.current.notify(tr('La riunione è cambiata: l’ho riletta.', 'The meeting changed, so I read it again.'));
      else latest.current.notify(errorText(error), true);
      try {
        await reread();
      } catch {
        /* The page keeps what it had. */
      }
      return false;
    } finally {
      setBusy(false);
      void keepFocusInDetail();
    }
  }
  const toggle = (action: MeetingAction, done: boolean) =>
    run('meeting_toggle_action', { actionId: action.id, done, expectedRevision: latest.current.detail.revision });
  const cancel = (job: MeetingJob) => run('meeting_cancel_job', { jobId: job.id });
  const retry = (jobId: string) => run('meeting_retry_job', { jobId });
  const transcribe = () => run('meeting_transcribe_default');
  /**
   * The dialog closes once the command was sent, whatever it answered (a refusal has been shown and
   * the meeting read again by `run`). If another command was still on its way nothing was sent: the
   * dialog stays, so the press is not swallowed, and can be made again.
   */
  async function discard() {
    if ((await run('meeting_discard_audio')) !== null) {
      setDiscarding(false);
      void keepFocusInDetail();
    }
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      latest.current.notify(tr('Copiato', 'Copied'));
    } catch {
      latest.current.notify(tr('Non sono riuscito a copiare.', 'Could not copy.'), true);
    }
  }

  const audioNote = {
    removed: tr("L'audio è stato eliminato dopo la trascrizione.", 'The audio was deleted after it was transcribed.'),
    empty_transcript: tr(
      "Trascrizione vuota: l'audio è conservato. Controlla i dispositivi in Impostazioni.",
      'Empty transcript: the audio is kept. Check your devices in Settings.',
    ),
    interrupted_capture: tr(
      "Registrazione con interruzioni: l'audio è conservato.",
      'Recording with gaps: the audio is kept.',
    ),
  };
  const ariaBusy = busy ? 'true' : undefined;

  let body;
  if (current.kind === 'live') body = <MeetingLive detail={detail} live={live} notify={notify} onchange={onchange} />;
  else if (current.kind === 'processing') {
    const job = current.job;
    body = <MeetingProgress job={job} busy={busy} oncancel={() => void cancel(job)} />;
  } else if (current.kind === 'recover')
    body = (
      <MeetingNotice
        title={tr('La registrazione si è interrotta', 'The recording was interrupted')}
        text={tr(
          "Qamille si è chiuso mentre registravi. L'audio salvato fino a quel momento si può recuperare.",
          'Qamille closed while you were recording. The audio saved until then can be recovered.',
        )}
        note={tr('Dopo il recupero la trascrizione parte da sola.', 'After recovery the transcript starts by itself.')}
        actions={
          <Button variant="primary" aria-busy={ariaBusy} onClick={() => void run('meeting_recover')}>
            <Icon name="refresh" size={15} />
            {tr('Recupera audio', 'Recover audio')}
          </Button>
        }
      />
    );
  else if (current.kind === 'no_transcript')
    body = (
      <MeetingNoTranscript
        detail={detail}
        reason={current.reason}
        message={current.message}
        failedJob={current.failedJob}
        busy={busy}
        ontranscribe={() => void transcribe()}
        ondiscard={() => setDiscarding(true)}
        onsettings={() => onsettings('Meetings')}
      />
    );
  else
    body = (
      <>
        {current.audio && (
          <div className="audio-note">
            <Icon name="info" size={15} />
            <span>{audioNote[current.audio]}</span>
            {current.audio !== 'removed' && (
              <Button variant="secondary" onClick={() => setDiscarding(true)}>
                {tr('Elimina audio', 'Delete audio')}
              </Button>
            )}
          </div>
        )}
        <div className="detail-tabs">
          <Tabs
            value={tab}
            onChange={(value) => ontab(value === 'transcript' ? 'transcript' : 'summary')}
            aria-label={tr('Contenuto della riunione', 'Meeting content')}
            items={[
              { value: 'summary', label: tr('Sintesi', 'Summary') },
              { value: 'transcript', label: tr('Trascrizione', 'Transcript') },
            ]}
            ids={{ trigger: (value) => `meeting-tab-${value}`, content: (value) => `meeting-panel-${value}` }}
          />
        </div>
        <div className="detail-body" role="tabpanel" id={`meeting-panel-${tab}`} aria-labelledby={`meeting-tab-${tab}`}>
          {tab === 'summary' ? (
            <MeetingSummary
              detail={detail}
              view={current.summary}
              busy={busy}
              oncopy={() => void copy(summaryPlainText(detail))}
              ontoggle={(action, done) => void toggle(action, done)}
              onretry={(jobId) => void retry(jobId)}
              oncancel={(job) => void cancel(job)}
              ongenerate={() => void run('meeting_summarize_default')}
              onsettings={() => onsettings('Assistant')}
            />
          ) : (
            <MeetingTranscript
              detail={detail}
              highlight={highlight}
              oncopy={() => void copy(transcriptPlainText(detail))}
            />
          )}
        </div>
      </>
    );

  return (
    <>
      <div className="detail-view" aria-busy={ariaBusy}>
        <MeetingHeader
          detail={detail}
          event={event}
          menu={current.kind !== 'live' && current.kind !== 'processing'}
          toRecover={current.kind === 'recover'}
          notify={notify}
          onchange={onchange}
          ondeleted={ondeleted}
        />
        {body}
      </div>

      {discarding && (
        <Modal
          size="sm"
          className="meeting-discard-dialog"
          aria-label={tr('Elimina audio', 'Delete audio')}
          onClose={() => setDiscarding(false)}
        >
          <h2>{tr('Elimina audio', 'Delete audio')}</h2>
          <p className="confirm-text">
            {tr(
              "L'audio di questa riunione verrà eliminato e non si potrà più trascrivere di nuovo.",
              'The audio of this meeting will be deleted and it can no longer be transcribed again.',
            )}
          </p>
          <ModalFooter>
            <Button variant="secondary" onClick={() => setDiscarding(false)}>
              {tr('Annulla', 'Cancel')}
            </Button>
            <Button variant="primary" aria-busy={ariaBusy} onClick={() => void discard()}>
              {tr('Elimina audio', 'Delete audio')}
            </Button>
          </ModalFooter>
        </Modal>
      )}
    </>
  );
}
