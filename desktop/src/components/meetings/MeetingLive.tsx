// The meeting being recorded: the time, the levels of the two sources, "Pausa"/"Riprendi" and
// "Termina". Nothing else. The time and the levels come from the capture the indicator already
// polls once a second (the shell hands it down as `live`); this component asks for nothing. The
// buttons answer with the new capture, which goes back on the bus so every reader hears it, and
// the meeting is read again (after "Termina" it is a job in progress). The buttons are not disabled
// while the answer is awaited (a disabled button drops the keyboard focus for good): a press during
// that time is ignored, and the panel says `aria-busy`.
import { useRef, useState } from 'react';
import Button from '../../ui/Button';
import Icon from '../../ui/Icon';
import { errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';
import { meetingCall, publishCapture, stateLabel, type CaptureSnapshot, type MeetingDetail } from '../../lib/meetings';
import { keepFocusInDetail } from '../../lib/meetings-focus';
import { clockLabel } from '../../lib/meetings-view';
import './MeetingLive.css';

export type MeetingLiveProps = {
  detail: MeetingDetail;
  /** The capture the bus holds. It is another meeting's, or none, until the first poll. */
  live: CaptureSnapshot;
  notify: (message: string, error?: boolean) => void;
  onchange: (detail: MeetingDetail) => void;
};

const BARS = 8;
const bars = Array.from({ length: BARS }, (_, index) => index);
/** How many of the bars are lit: the level is a power, so a quiet voice still shows. */
const lit = (level: number) => Math.min(BARS, Math.ceil(Math.sqrt(Math.max(0, level)) * BARS));

export default function MeetingLive({ detail, live, notify, onchange }: MeetingLiveProps) {
  /** The bus when it is about this meeting; until then, what the meeting itself was read with. */
  const capture = live.meeting_id === detail.meeting.id ? live : detail.capture;
  const paused = capture.state === 'paused';
  const settled = capture.state === 'recording' || paused;
  const label = capture.state === 'recording' ? tr('In registrazione', 'Recording') : stateLabel(capture.state);
  const sources = [
    { name: tr('Microfono', 'Microphone'), level: paused ? 0 : capture.microphone_level },
    { name: tr('Audio del computer', 'Computer audio'), level: paused ? 0 : capture.system_level },
  ];

  const [busy, setBusyState] = useState(false);
  const busyRef = useRef(false);
  const setBusy = (value: boolean) => {
    busyRef.current = value;
    setBusyState(value);
  };
  // Read after an await: the props as they are now.
  const latest = useRef({ detail, notify, onchange });
  latest.current = { detail, notify, onchange };

  async function control(command: 'meeting_pause' | 'meeting_resume' | 'meeting_stop') {
    if (busyRef.current) return;
    setBusy(true);
    try {
      publishCapture(await meetingCall<CaptureSnapshot>(command));
      latest.current.onchange(
        await meetingCall<MeetingDetail>('meeting_read', { id: latest.current.detail.meeting.id }),
      );
    } catch (error) {
      latest.current.notify(errorText(error), true);
    } finally {
      setBusy(false);
      void keepFocusInDetail();
    }
  }

  return (
    <div className="live" aria-busy={busy ? 'true' : undefined}>
      <span className={paused ? 'live-status paused' : 'live-status'}>{label}</span>
      <div className="live-clock" role="timer" aria-label={tr('Tempo trascorso', 'Elapsed time')}>
        {clockLabel(capture.elapsed_ms)}
      </div>
      <div className="live-levels">
        {sources.map((source) => (
          <div className="live-level" key={source.name}>
            <span
              className="meter"
              role="meter"
              aria-label={source.name}
              aria-valuemin={0}
              aria-valuemax={1}
              aria-valuenow={source.level}
            >
              {bars.map((index) => (
                <b
                  key={index}
                  className={index < lit(source.level) ? 'on' : undefined}
                  style={{ height: `${6 + index * 3.5}px` }}
                />
              ))}
            </span>
            {source.name}
          </div>
        ))}
      </div>
      <div className="live-buttons">
        <Button
          variant="secondary"
          disabled={!settled}
          onClick={() => void control(paused ? 'meeting_resume' : 'meeting_pause')}
        >
          <Icon name={paused ? 'play' : 'pause'} size={15} />
          {paused ? tr('Riprendi', 'Resume') : tr('Pausa', 'Pause')}
        </Button>
        <Button variant="primary" onClick={() => void control('meeting_stop')}>
          <Icon name="stop" size={15} />
          {tr('Termina', 'Stop')}
        </Button>
      </div>
      <p className="live-note">
        {tr(
          'Dopo «Termina» la trascrizione e la sintesi partono da sole.',
          'After “Stop” the transcript and the summary start by themselves.',
        )}
      </p>
    </div>
  );
}
