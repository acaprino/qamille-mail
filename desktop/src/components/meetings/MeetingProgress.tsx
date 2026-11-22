// A job that is running: what it is doing, how far it is, and "Annulla". Nothing else. In the
// page it replaces the tabs (the first transcription); `compact` is the same thing inside the
// Sintesi tab, while the transcript next to it stays readable. It draws what the meeting says
// now; the page re-reads the meeting, this does not ask for anything. The block fades in and the
// bar grows with a spring (no movement under prefers-reduced-motion).
import Button from '../../ui/Button';
import Progress from '../../ui/Progress';
import { useAppear } from '../../ui/motion';
import { tr } from '../../lib/i18n';
import type { MeetingJob } from '../../lib/meetings';
import MeetingNotice from './MeetingNotice';
import './MeetingProgress.css';

export type MeetingProgressProps = {
  job: MeetingJob;
  compact?: boolean;
  busy?: boolean;
  oncancel: () => void;
};

export default function MeetingProgress({ job, compact = false, busy = false, oncancel }: MeetingProgressProps) {
  const appear = useAppear();
  const transcribing = job.kind === 'transcribe';
  const heading = transcribing
    ? tr('Sto trascrivendo la riunione', 'Transcribing the meeting')
    : tr('Sto scrivendo la sintesi', 'Writing the summary');
  /** The same words as the row of the list. */
  const phase = transcribing
    ? tr(`Trascrizione ${job.progress}%`, `Transcribing ${job.progress}%`)
    : tr('Sintesi…', 'Summarizing…');
  const name = transcribing ? tr('Trascrizione', 'Transcription') : tr('Sintesi', 'Summary');

  const bar = <Progress value={job.progress} aria-label={name} size="sm" />;
  const cancel = (
    <Button variant="secondary" aria-busy={busy ? 'true' : undefined} onClick={oncancel}>
      {tr('Annulla', 'Cancel')}
    </Button>
  );

  return compact ? (
    <div ref={appear} className="inline meeting-progress">
      <h3>{heading}</h3>
      <p className="phase">{phase}</p>
      {bar}
      <div>{cancel}</div>
    </div>
  ) : (
    <MeetingNotice title={heading} actions={cancel} className="meeting-progress">
      <p className="phase">{phase}</p>
      {bar}
      <p className="reassure">
        {tr('Puoi lasciare la pagina: il lavoro continua.', 'You can leave the page: the work goes on.')}
      </p>
    </MeetingNotice>
  );
}
