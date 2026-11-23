// Settings > Meetings: how a recording is transcribed (language, engine, model), which devices it
// uses, the meeting reminders and the one-shot audio cleanup. It edits the `meeting_*` fields of
// the Settings form (handed back through `onSettingsChange`), which the Save bar of Settings saves.
// The transcription models live on the server: the app installs nothing.
import { useEffect, useRef } from 'react';
import Button from '../ui/Button';
import Field from '../ui/Field';
import Modal from '../ui/Modal';
import Select from '../ui/Select';
import Switch from '../ui/Switch';
import { errorText } from '../lib/api';
import { backendText, tr } from '../lib/i18n';
import {
  meetingCall,
  type AudioCleanup,
  type AudioDevice,
  type SpeechModel,
} from '../lib/meetings';
import { meetingEngine } from '../lib/meeting-engine';
import { isMicrophone, isSystemAudio } from '../lib/meetings-view';
import { regionalLocale } from '../lib/regional';
import { useReactive } from '../lib/store';
import type { Settings } from '../lib/types';
import './MeetingSettings.css';
import { Input } from '../ui/Input';

export type MeetingSettingsProps = {
  settings: Settings;
  /** The form with one field changed (was `bind:settings`). */
  onSettingsChange?: (value: Settings) => void;
  notify: (message: string, error?: boolean) => void;
};

// Select reads '' as "nothing chosen", so "System default" (saved as '') has a value of its own.
const SYSTEM_DEFAULT = '__system_default__';
const MB = 1_048_576;

type Item = { value: string; label: string };
/** A saved choice the list does not have (an older version, an edited file) is still the choice: show its code. */
function withSaved(items: Item[], saved: string): Item[] {
  return saved && !items.some((item) => item.value === saved) ? [...items, { value: saved, label: saved }] : items;
}
function megabytes(bytes: number): string {
  if (bytes < 100_000) return tr('meno di 0,1 MB', 'less than 0.1 MB');
  return `${(bytes / MB).toLocaleString(regionalLocale(), {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  })} MB`;
}

export default function MeetingSettings({ settings, onSettingsChange, notify }: MeetingSettingsProps) {
  // Read after awaits and in the download timer: a deep-reactive store is always current.
  const s = useReactive(() => ({
    devices: [] as AudioDevice[],
    models: [] as SpeechModel[],
    loaded: false,
    loadError: '',
    preview: null as AudioCleanup | null,
    cleaning: false,
  }));
  const alive = useRef(true);
  const latest = useRef({ settings, onSettingsChange, notify });
  latest.current = { settings, onSettingsChange, notify };
  /** One field of the form changes; the rest stays as it is. */
  const set = (patch: Partial<Settings>) =>
    latest.current.onSettingsChange?.({ ...latest.current.settings, ...patch });

  const chosen = s.models.find((model) => model.id === settings.meeting_model);

  const languageItems = withSaved(
    [
      { value: 'it', label: 'Italiano' },
      { value: 'en', label: 'English' },
      { value: 'de', label: 'Deutsch' },
      { value: 'es', label: 'Español' },
      { value: 'fr', label: 'Français' },
    ],
    settings.meeting_language,
  );
  // «Server di calcolo» follows Settings > Compute (this computer or the remote server); the
  // external engine is the OpenAI-compatible endpoint of Assistant. Older names read as these.
  const engine = meetingEngine(settings.meeting_engine);
  const hasServer = settings.compute_mode === 'server';
  const engineItems = [
    { value: 'compute', label: tr('Server di calcolo', 'Compute server') },
    { value: 'external', label: tr('Esterno compatibile OpenAI', 'External OpenAI-compatible') },
  ];
  const modelItems = (() => {
    const items = s.models.map((model) => ({
      value: model.id,
      label: `${model.name} · ${model.installed ? tr('installato', 'installed') : `${Math.round(model.bytes / MB)} MB`}`,
    }));
    // Before the list is read every model would look unknown.
    return s.loaded ? withSaved(items, settings.meeting_model) : items;
  })();
  const leadItems = [1, 2, 5, 10].map((minutes) => ({
    value: String(minutes),
    label: tr(
      `${minutes} ${minutes === 1 ? 'minuto' : 'minuti'} prima`,
      `${minutes} ${minutes === 1 ? 'minute' : 'minutes'} before`,
    ),
  }));
  function deviceItems(wanted: (device: AudioDevice) => boolean, saved: string) {
    const items = [
      { value: SYSTEM_DEFAULT, label: tr('Predefinito di sistema', 'System default') },
      ...s.devices.filter(wanted).map((device) => ({ value: device.id, label: device.name })),
    ];
    // A device that is no longer there is still the saved choice: say so instead of showing nothing.
    if (s.loaded && saved && !items.some((item) => item.value === saved))
      items.push({ value: saved, label: tr('Dispositivo non collegato', 'Device not connected') });
    return items;
  }
  const microphoneItems = deviceItems(isMicrophone, settings.meeting_microphone);
  const systemItems = deviceItems(isSystemAudio, settings.meeting_system_audio);

  useEffect(() => {
    alive.current = true;
    void (async () => {
      const [available, speech] = await Promise.allSettled([
        meetingCall<AudioDevice[]>('meeting_devices'),
        meetingCall<SpeechModel[]>('meeting_models'),
      ]);
      if (!alive.current) return;
      if (available.status === 'fulfilled') s.devices = available.value;
      if (speech.status === 'fulfilled') s.models = speech.value;
      const failed = [available, speech].find((r) => r.status === 'rejected');
      if (failed?.status === 'rejected') s.loadError = errorText(failed.reason);
      s.loaded = true;
    })();
    return () => {
      alive.current = false;
    };
  }, []);
  const nothingToDelete = () => latest.current.notify(tr('Non c’è audio da eliminare.', 'There is no audio to delete.'));
  async function askCleanup() {
    if (s.cleaning) return;
    s.cleaning = true;
    try {
      const due = await meetingCall<AudioCleanup>('meeting_audio_cleanup_preview');
      if (due.meetings > 0) s.preview = due;
      else nothingToDelete();
    } catch (e) {
      latest.current.notify(errorText(e), true);
    } finally {
      s.cleaning = false;
    }
  }
  async function confirmCleanup() {
    if (s.cleaning) return;
    s.cleaning = true;
    try {
      const done = await meetingCall<AudioCleanup>('meeting_audio_cleanup');
      s.preview = null;
      if (done.meetings > 0)
        latest.current.notify(
          tr(`Audio eliminato: ${megabytes(done.bytes)} liberati`, `Audio deleted: ${megabytes(done.bytes)} freed`),
        );
      else nothingToDelete();
    } catch (e) {
      s.preview = null;
      latest.current.notify(errorText(e), true);
    } finally {
      s.cleaning = false;
    }
  }

  const preview = s.preview;
  return (
    <>
      <h2>{tr('Riunioni', 'Meetings')}</h2>
      <p className="muted">
        {tr(
          'Come Qamille trascrive le registrazioni, quali dispositivi usa e quando ti avvisa.',
          'How Qamille transcribes recordings, which devices it uses and when it reminds you.',
        )}
      </p>
      {s.loadError && (
        <p className="meeting-error" role="alert">
          {s.loadError}
        </p>
      )}

      <h3>{tr('Trascrizione', 'Transcription')}</h3>
      <Select
        className="settings-select"
        label={tr('Lingua della trascrizione', 'Transcription language')}
        value={settings.meeting_language}
        items={languageItems}
        onChange={(value) => set({ meeting_language: value })}
      />
      <Select
        className="settings-select"
        label={tr('Motore di trascrizione', 'Transcription engine')}
        value={engine}
        items={engineItems}
        onChange={(value) => set({ meeting_engine: meetingEngine(value) })}
      />
      {engine === 'compute' ? (
        <>
          <p className="field-help">
            {hasServer
              ? tr(
                  'Trascrive il server indicato in Impostazioni → Calcolo: l’audio delle riunioni viene inviato a quel server.',
                  'The server set in Settings → Compute transcribes: the meeting audio is sent to that server.',
                )
              : tr(
                  'Senza server le registrazioni restano da trascrivere e si trascrivono quando ne colleghi uno.',
                  'Without a server the recordings wait to be transcribed and are transcribed when you connect one.',
                )}
          </p>
          <Select
            className="settings-select"
            label={tr('Modello', 'Model')}
            value={settings.meeting_model}
            items={modelItems}
            onChange={(value) => set({ meeting_model: value })}
          />
          {hasServer && s.loaded && chosen && !chosen.installed && (
            <p className="field-help">
              {tr(
                'Questo modello non è installato sul server: lo installa chi lo amministra.',
                'This model is not installed on the server: its administrator installs it.',
              )}
            </p>
          )}
        </>
      ) : (
        <>
          <Field>
            {tr('Modello remoto', 'Remote model')}
            <Input
              value={settings.meeting_remote_model}
              onChange={(event) => set({ meeting_remote_model: event.currentTarget.value })}
              placeholder="whisper-1"
              autoComplete="off"
              spellCheck={false}
              aria-describedby={
                settings.ai_provider === 'OpenAiCompatible'
                  ? 'meeting-remote-help'
                  : 'meeting-remote-help meeting-remote-missing'
              }
            />
          </Field>
          <p className="field-help" id="meeting-remote-help">
            {tr(
              "L'audio viene inviato al provider compatibile con OpenAI impostato in Assistente, con la sua chiave API. Indica un modello audio disponibile su quel servizio.",
              'The audio is sent to the OpenAI-compatible provider set in Assistant, with its API key. Enter an audio model that service offers.',
            )}
          </p>
          {settings.ai_provider !== 'OpenAiCompatible' && (
            <p className="field-help meeting-error" id="meeting-remote-missing">
              {tr(
                'In Assistente non c’è ancora un provider compatibile con OpenAI: la trascrizione remota non parte finché non lo imposti.',
                'Assistant has no OpenAI-compatible provider yet: remote transcription cannot start until you set one.',
              )}
            </p>
          )}
        </>
      )}

      <h3>{tr('Dispositivi', 'Devices')}</h3>
      <Select
        className="settings-select"
        label={tr('Microfono', 'Microphone')}
        value={settings.meeting_microphone || SYSTEM_DEFAULT}
        items={microphoneItems}
        onChange={(value) => set({ meeting_microphone: value === SYSTEM_DEFAULT ? '' : value })}
      />
      <Select
        className="settings-select"
        label={tr('Audio del computer', 'Computer audio')}
        value={settings.meeting_system_audio || SYSTEM_DEFAULT}
        items={systemItems}
        onChange={(value) => set({ meeting_system_audio: value === SYSTEM_DEFAULT ? '' : value })}
      />
      <p className="field-help">
        {tr(
          'Il microfono registra la tua voce, l’audio del computer le altre persone della chiamata.',
          'The microphone records your voice, the computer audio the other people on the call.',
        )}
      </p>

      <h3>{tr('Promemoria', 'Reminders')}</h3>
      <div className="setting-row">
        <Switch checked={settings.meeting_reminders} onChange={(checked) => set({ meeting_reminders: checked })}>
          <strong>{tr('Promemoria riunioni', 'Meeting reminders')}</strong>
        </Switch>
        <p>
          {tr(
            'Prima di una riunione online compare in alto sullo schermo un avviso per entrare o entrare e registrare. Tieni aperto Qamille, anche ridotto a icona.',
            'Before an online meeting, a chip at the top of the screen lets you join, or join and record. Keep Qamille open, even minimized.',
          )}
        </p>
      </div>
      <Select
        className="settings-select"
        label={tr('Anticipo del promemoria', 'Reminder time')}
        value={String(settings.reminder_lead_minutes)}
        items={leadItems}
        disabled={!settings.meeting_reminders}
        onChange={(value) => set({ reminder_lead_minutes: Number(value) })}
      />

      <h3>{tr('Audio delle riunioni già trascritte', 'Audio of meetings already transcribed')}</h3>
      <p className="muted">
        {tr(
          'Le nuove registrazioni perdono l’audio da sole appena la trascrizione è completa. Qui puoi eliminare ora l’audio di quelle trascritte prima. Le trascrizioni restano.',
          'New recordings lose their audio by themselves once the transcript is complete. Here you can delete the audio of those transcribed before. The transcripts stay.',
        )}
      </p>
      <Button
        variant="secondary"
        aria-disabled={s.cleaning ? 'true' : undefined}
        aria-busy={s.cleaning ? 'true' : undefined}
        onClick={askCleanup}
      >
        {tr("Elimina l'audio…", 'Delete audio…')}
      </Button>

      {preview && (
        <Modal
          size="sm"
          aria-labelledby="meeting-cleanup-title"
          onClose={() => {
            if (!s.cleaning) s.preview = null;
          }}
        >
          <h2 id="meeting-cleanup-title">
            {tr(
              `Eliminare l’audio di ${preview.meetings} ${preview.meetings === 1 ? 'riunione' : 'riunioni'}?`,
              `Delete the audio of ${preview.meetings} ${preview.meetings === 1 ? 'meeting' : 'meetings'}?`,
            )}
          </h2>
          <p>
            {tr(
              `Si liberano ${megabytes(preview.bytes)}. Le trascrizioni restano. Non si può annullare.`,
              `This frees ${megabytes(preview.bytes)}. Transcripts stay. This cannot be undone.`,
            )}
          </p>
          <div className="modal-actions">
            <Button
              variant="secondary"
              aria-disabled={s.cleaning ? 'true' : undefined}
              onClick={() => {
                if (!s.cleaning) s.preview = null;
              }}
            >
              {tr('Annulla', 'Cancel')}
            </Button>
            <Button
              variant="primary"
              className="danger"
              aria-disabled={s.cleaning ? 'true' : undefined}
              aria-busy={s.cleaning ? 'true' : undefined}
              onClick={confirmCleanup}
            >
              {tr('Elimina', 'Delete')}
            </Button>
          </div>
        </Modal>
      )}
    </>
  );
}
