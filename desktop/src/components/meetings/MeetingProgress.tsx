// A job that is running: what it is doing, how far it is, and "Annulla". Nothing else. In the
// page it replaces the tabs (the first transcription); `compact` is the same thing inside the
// Sintesi tab, while the transcript next to it stays readable. It draws what the meeting says
// now; the page re-reads the meeting, this does not ask for anything. The block fades in and the
// bar grows with a spring (no movement under prefers-reduced-motion).
import { motion, useReducedMotion } from '../../ui/motion';
import Button from '../../ds/primitives/Button';
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

const spring = { type: 'spring', stiffness: 400, damping: 32 } as const;

export default function MeetingProgress({ job, compact = false, busy = false, oncancel }: MeetingProgressProps) {
  const still = useReducedMotion();
  const transcribing = job.kind === 'transcribe';
  const heading = transcribing
    ? tr('Sto trascrivendo la riunione', 'Transcribing the meeting')
    : tr('Sto scrivendo la sintesi', 'Writing the summary');
  /** The same words as the row of the list. */
  const phase = transcribing
    ? tr(`Trascrizione ${job.progress}%`, `Transcribing ${job.progress}%`)
    : tr('Sintesi…', 'Summarizing…');
  const name = transcribing ? tr('Trascrizione', 'Transcription') : tr('Sintesi', 'Summary');

  const bar = (
    <div
      className="bar"
      role="progressbar"
      aria-label={name}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={job.progress}
    >
      <motion.span
        initial={false}
        animate={{ width: `${job.progress}%` }}
        transition={still ? { duration: 0 } : spring}
      />
    </div>
  );
  const cancel = (
    <Button variant="secondary" aria-busy={busy ? 'true' : undefined} onClick={oncancel}>
      {tr('Annulla', 'Cancel')}
    </Button>
  );

  return compact ? (
    <motion.div
      className="inline meeting-progress"
      initial={still ? false : { opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={spring}
    >
      <h3>{heading}</h3>
      <p className="phase">{phase}</p>
      {bar}
      <div>{cancel}</div>
    </motion.div>
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
