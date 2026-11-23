// "Registra ora": the way to record a meeting that is not in the calendar (in person, an
// improvised call). A small popover with an optional name; the sound is the microphone and the
// computer audio, with the devices from Settings or the default ones, as `record_meeting` does for
// a call joined from the calendar. Nothing else to choose: devices are not part of this page.
// While a recording is going on the button waits, and a line under it says why (a disabled button
// cannot show a hint on hover, nor take the focus a keyboard user would need to read one). It is
// `aria-disabled`, not `disabled`: the popover gives the focus back to its trigger when a start
// closes it, and the recording that start began would otherwise disable the trigger under it
// and throw the focus to the page.
import { useRef, useState } from 'react';
import Button from '../../ui/Button';
import Field from '../../ui/Field';
import Icon from '../../ui/Icon';
import Popover from '../../ui/Popover';
import { errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';
import {
  captureActive,
  meetingCall,
  type AudioDevice,
  type CaptureSnapshot,
  type MeetingDetail,
} from '../../lib/meetings';
import { pickDevices } from '../../lib/meetings-view';
import './RecordNow.css';
import { Input } from '../../ui/Input';

export type RecordNowProps = {
  settings: { meeting_microphone: string; meeting_system_audio: string };
  /** One recording at a time: while one is going on the button waits. */
  live: CaptureSnapshot;
  notify: (message: string, error?: boolean) => void;
  onstarted: (detail: MeetingDetail) => void;
};

export default function RecordNow({ settings, live, notify, onstarted }: RecordNowProps) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  // `start` reads these after an await: through refs they are the current ones.
  const busyRef = useRef(false);
  const latest = useRef({ settings, notify, onstarted, name });
  latest.current = { settings, notify, onstarted, name };

  const recording = captureActive(live);
  const chosenInSettings = Boolean(settings.meeting_microphone || settings.meeting_system_audio);

  async function start() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try {
      const devices = await meetingCall<AudioDevice[]>('meeting_devices');
      const { microphone, system } = pickDevices(devices, latest.current.settings);
      if (!microphone && !system) {
        latest.current.notify(
          tr(
            'Non trovo né un microfono né l’audio del computer. Controlla i dispositivi.',
            'No microphone or computer audio found. Check your audio devices.',
          ),
          true,
        );
        return;
      }
      // An empty name is fine: the meeting is named after the day and time, and can be renamed.
      const detail = await meetingCall<MeetingDetail>('meeting_start', {
        request: {
          request_id: crypto.randomUUID(),
          title: latest.current.name.trim(),
          event_id: null,
          microphone,
          system,
        },
      });
      setOpen(false);
      setName('');
      latest.current.onstarted(detail);
    } catch (error) {
      latest.current.notify(errorText(error), true);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  return (
    <>
      <Popover
        open={open}
        onOpenChange={(next) => setOpen(next && !recording)}
        label={tr('Registra ora', 'Record now')}
        className="record-popover"
        trigger={
          <Button
            variant="primary"
            className="record-trigger"
            aria-disabled={recording ? 'true' : undefined}
            aria-describedby={recording ? 'record-now-why' : undefined}
            // Runs before the popover's own handler: a prevented click does not open it.
            onClick={(event) => {
              if (recording) event.preventDefault();
            }}
          >
            <Icon name="mic" size={15} />
            {tr('Registra ora', 'Record now')}
          </Button>
        }
      >
        <form
          className="record-form"
          onSubmit={(event) => {
            event.preventDefault();
            void start();
          }}
        >
          <Field>
            <span>{tr('Nome (facoltativo)', 'Name (optional)')}</span>
            <Input
              value={name}
              onChange={(event) => setName(event.currentTarget.value)}
              maxLength={300}
              autoComplete="off"
              placeholder={tr('Es. Chiamata con il fornitore', 'E.g. Call with the supplier')}
            />
          </Field>
          <p className="record-sources">
            <Icon name="mic" size={15} />
            {chosenInSettings
              ? tr(
                  'Microfono e audio del computer, dispositivi scelti in Impostazioni',
                  'Microphone and computer audio, devices chosen in Settings',
                )
              : tr('Microfono e audio del computer, dispositivi predefiniti', 'Microphone and computer audio, default devices')}
          </p>
          <div className="record-actions">
            <Button
              type="button"
              variant="secondary"
              onClick={() => {
                setOpen(false);
                setName('');
              }}
            >
              {tr('Annulla', 'Cancel')}
            </Button>
            {/* Not `disabled` while the start is on its way: that would drop the focus of whoever pressed
                Enter. `start` ignores a second submit, and the button says it is busy. */}
            <Button
              type="submit"
              variant="primary"
              aria-disabled={busy ? 'true' : undefined}
              aria-busy={busy ? 'true' : undefined}
            >
              <Icon name="mic" size={15} />
              {tr('Registra', 'Record')}
            </Button>
          </div>
        </form>
      </Popover>
      {recording && (
        <p className="record-why" id="record-now-why">
          {tr('Una registrazione è già in corso', 'A recording is already in progress')}
        </p>
      )}
    </>
  );
}
