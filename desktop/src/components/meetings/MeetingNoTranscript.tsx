// A meeting without a transcript, and why: the audio was deleted (nothing left to do), the
// transcript came out empty (the audio is kept: delete it or transcribe again), the last
// transcription failed or was cancelled ("Riprova"), the engine cannot run (the reason, a link to
// Settings and "Trascrivi"), or it was just never transcribed ("Trascrivi"). It decides what to
// say; the buttons call back and the page does the work. "Riprova" is a new transcription with the
// engine Settings says now, not the failed job run again: the job would restart with the options
// it failed with, and a fix made in Settings (the model, the language) would change nothing.
import Button from '../../ui/Button';
import Icon from '../../ui/Icon';
import { backendText, tr } from '../../lib/i18n';
import type { MeetingDetail, MeetingJob } from '../../lib/meetings';
import MeetingNotice from './MeetingNotice';

export type MeetingNoTranscriptProps = {
  detail: MeetingDetail;
  reason: 'blocked' | 'failed' | 'empty' | 'never';
  message: string | null;
  failedJob: MeetingJob | null;
  busy: boolean;
  ontranscribe: () => void;
  ondiscard: () => void;
  onsettings: () => void;
};

export default function MeetingNoTranscript({
  detail,
  reason,
  message,
  failedJob,
  busy,
  ontranscribe,
  ondiscard,
  onsettings,
}: MeetingNoTranscriptProps) {
  const cancelled = failedJob?.state === 'cancelled';
  const NO_TRANSCRIPT = tr('Questa riunione non ha ancora una trascrizione', 'This meeting has no transcript yet');
  const ariaBusy = busy ? 'true' : undefined;

  if (detail.audio_removed)
    return (
      <MeetingNotice
        title={tr('Nessuna trascrizione', 'No transcript')}
        text={tr(
          'L’audio è stato eliminato: non c’è più nulla da trascrivere.',
          'The audio was deleted: there is nothing left to transcribe.',
        )}
      />
    );
  if (reason === 'empty')
    return (
      <MeetingNotice
        title={tr('Nessun testo trascritto', 'No text was transcribed')}
        message={tr(
          "Trascrizione vuota: l'audio è conservato. Controlla i dispositivi in Impostazioni.",
          'Empty transcript: the audio is kept. Check your devices in Settings.',
        )}
        actions={
          <>
            <Button variant="secondary" onClick={ondiscard}>
              {tr('Elimina audio', 'Delete audio')}
            </Button>
            <Button variant="primary" aria-busy={ariaBusy} onClick={ontranscribe}>
              <Icon name="mic" size={15} />
              {tr('Trascrivi di nuovo', 'Transcribe again')}
            </Button>
          </>
        }
      />
    );
  if (reason === 'failed')
    return (
      <MeetingNotice
        title={NO_TRANSCRIPT}
        tone={cancelled ? 'info' : 'danger'}
        message={
          message?.trim()
            ? backendText(message)
            : cancelled
              ? tr('La trascrizione è stata annullata.', 'The transcript was cancelled.')
              : tr('La trascrizione non è riuscita.', 'The transcript did not work.')
        }
        actions={
          <Button variant="primary" aria-busy={ariaBusy} onClick={ontranscribe}>
            <Icon name="refresh" size={15} />
            {tr('Riprova', 'Try again')}
          </Button>
        }
      />
    );
  return (
    <MeetingNotice
      title={NO_TRANSCRIPT}
      message={reason === 'blocked' && message ? backendText(message) : undefined}
      note={
        reason === 'blocked'
          ? tr(
              'Le riunioni vecchie non partono da sole quando installi il modello: scegli tu quali trascrivere.',
              'Old meetings do not start by themselves when you install the model: you choose which to transcribe.',
            )
          : undefined
      }
      actions={
        <>
          <Button variant="primary" aria-busy={ariaBusy} onClick={ontranscribe}>
            <Icon name="mic" size={15} />
            {tr('Trascrivi', 'Transcribe')}
          </Button>
          {reason === 'blocked' && (
            <Button variant="secondary" onClick={onsettings}>
              {tr('Apri Impostazioni › Riunioni', 'Open Settings › Meetings')}
            </Button>
          )}
        </>
      }
    />
  );
}
