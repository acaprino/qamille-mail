// The floating chip of the recording going on (or of one that was interrupted and can be
// recovered), at the bottom of the window on every page. It is the only poller of the capture: it
// reads the status every second and publishes it on the capture bus, which the Meetings page
// listens to. A click opens that meeting on the Meetings page. While it records, the microphone
// breathes (Motion), still when the system asks for reduced motion.
import { motion, useReducedMotion } from '../../ui/motion';
import { useEffect, useRef, useState } from 'react';
import Icon from '../../ui/Icon';
import { tr } from '../../lib/i18n';
import {
  captureActive,
  clockTime,
  emptyCapture,
  meetingCall,
  publishCapture,
  requestMeetingOpen,
  stateLabel,
  type CaptureSnapshot,
  type MeetingPage,
} from '../../lib/meetings';
import { cn } from '../../lib/utils';
import './MeetingRecordingIndicator.css';

export type MeetingRecordingIndicatorProps = {
  onopen: () => void;
  notify?: (message: string, error?: boolean) => void;
};

export default function MeetingRecordingIndicator({ onopen, notify }: MeetingRecordingIndicatorProps) {
  const still = useReducedMotion();
  const [capture, setCapture] = useState<CaptureSnapshot>(() => emptyCapture());
  // The poll reads these between awaits and in a timer: refs keep them current.
  const captureRef = useRef(capture);
  const notifyRef = useRef(notify);
  notifyRef.current = notify;

  function openCapture() {
    if (capture.meeting_id != null)
      requestMeetingOpen({
        meeting_id: capture.meeting_id,
        title: '',
        kind: 'notes',
        text: '',
        at_ms: null,
        version: null,
        segment_id: null,
      });
    onopen();
  }

  useEffect(() => {
    let alive = true;
    let pending = false;
    let failed = false;
    let previousLiveState = '';
    let recoveryCheckedAt = 0;
    let recoveryCapture: CaptureSnapshot | null = null;
    const show = (value: CaptureSnapshot) => {
      captureRef.current = value;
      setCapture(value);
    };
    const refresh = async () => {
      if (pending) return;
      pending = true;
      try {
        const value = await meetingCall<CaptureSnapshot>('meeting_capture_status');
        // This loop is the only poller of the capture; the Meetings page listens to what it reads.
        publishCapture(value);
        if (value.state === 'idle') {
          if (previousLiveState !== 'idle' || Date.now() - recoveryCheckedAt >= 5000) {
            const page = await meetingCall<MeetingPage>('meeting_list_page', {
              offset: 0,
              limit: 1,
              fromDate: null,
              toDate: null,
              state: 'interrupted',
            });
            const interrupted = page.items[0];
            recoveryCapture = interrupted ? { ...interrupted.capture, meeting_id: interrupted.meeting.id } : null;
            recoveryCheckedAt = Date.now();
          }
          if (alive) show(recoveryCapture || value);
        } else {
          recoveryCapture = null;
          if (alive) show(value);
        }
        previousLiveState = value.state;
        failed = false;
      } catch {
        const current = captureRef.current;
        if (!failed && (captureActive(current) || current.state === 'interrupted'))
          notifyRef.current?.(
            tr(
              'Impossibile aggiornare lo stato della registrazione. Apri Meeting per verificare.',
              "Couldn't update the recording status. Open Meeting to check.",
            ),
            true,
          );
        failed = true;
      } finally {
        pending = false;
      }
    };
    void refresh();
    const timer = setInterval(refresh, 1000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  const interrupted = capture.state === 'interrupted';
  if (!captureActive(capture) && !interrupted) return null;
  const breathing = !still && capture.state === 'recording';
  return (
    <button className={cn('recording-indicator', interrupted && 'interrupted')} onClick={openCapture}>
      <motion.span
        className="recording-mic"
        animate={breathing ? { opacity: [1, 0.4, 1] } : { opacity: 1 }}
        transition={breathing ? { duration: 1.6, ease: 'easeInOut', repeat: Infinity } : { duration: 0.2 }}
      >
        <Icon name="mic" size={16} />
      </motion.span>
      <span className="visually-hidden">
        {interrupted ? tr('Apri recupero registrazione:', 'Open recording recovery:') : tr('Apri registrazione:', 'Open recording:')}
      </span>
      <strong>{stateLabel(capture.state)}</strong>
      <span>{clockTime(capture.elapsed_ms)}</span>
      <span>{tr(`Su disco: ${clockTime(capture.saved_ms)}`, `On disk: ${clockTime(capture.saved_ms)}`)}</span>
      <span className="open">{interrupted ? tr('Apri Recupera', 'Open recovery') : tr('Apri Meeting', 'Open Meeting')}</span>
    </button>
  );
}
