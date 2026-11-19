import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Meeting } from './types';
import fixture from './sample.json';
import { tr } from './i18n';
import { call } from './api';
import { emptyCapture, lastCapture, publishCapture, subscribeCapture } from './capture-bus';

// The recording state is shared through the capture bus (its own module, so plain node can test it).
export { emptyCapture, lastCapture, publishCapture, subscribeCapture };

export type CaptureState =
  'idle' | 'preparing' | 'recording' | 'paused' | 'saving' | 'saved' | 'interrupted';
export interface AudioDevice {
  id: string;
  name: string;
  kind: string;
  is_default: boolean;
}
export interface AudioChunk {
  source: string;
  sequence: number;
  path: string;
  start_ms: number;
  end_ms: number;
  sample_rate: number;
  channels: number;
}
export interface CaptureSnapshot {
  meeting_id: number | null;
  state: CaptureState;
  elapsed_ms: number;
  saved_ms: number;
  microphone_level: number;
  system_level: number;
  chunks: AudioChunk[];
  error: string | null;
  warnings?: string[];
  missing_chunks?: AudioChunk[];
  incomplete?: boolean;
  gaps?: { source: string; start_ms: number; end_ms: number }[];
  requires_receipts?: boolean;
  unverified_chunks?: string[];
}
export interface TranscriptSegment {
  id: string;
  start_ms: number;
  end_ms: number;
  text: string;
  original_text: string;
  source: string;
  confidence: number | null;
}
export interface TranscriptVersion {
  id: string;
  created_at: number;
  model: string;
  language: string;
  segments: TranscriptSegment[];
  complete: boolean;
}
export interface MeetingNote {
  id: string;
  text: string;
  at_ms: number | null;
  revision: number;
}
export interface Evidence {
  kind: string;
  version: string;
  id: string;
}
export interface Decision {
  text: string;
  sources: Evidence[];
}
export interface MeetingAction {
  id: string;
  text: string;
  owner: string | null;
  due: string | null;
  done: boolean;
  sources: Evidence[];
}
export interface SummaryVersion {
  id: string;
  created_at: number;
  transcript_version: string | null;
  input_revision: number;
  language: string;
  model: string;
  summary: string;
  key_points: string[];
  decisions: Decision[];
  actions: MeetingAction[];
  complete: boolean;
}
export interface MeetingJob {
  id: string;
  kind: string;
  state: string;
  input_revision: number;
  progress: number;
  error: string | null;
  created_at: number;
  /** What a transcription was started with; a retry of the job runs with the same. */
  options?: TranscribeOptions | null;
  language?: string | null;
}
export interface MeetingDetail {
  schema_version: number;
  revision: number;
  meeting: Meeting;
  request_id: string;
  capture: CaptureSnapshot;
  transcripts: TranscriptVersion[];
  active_transcript: string | null;
  notes: MeetingNote[];
  note_history?: MeetingNote[];
  legacy_transcript?: string | null;
  legacy_notes?: string | null;
  summaries: SummaryVersion[];
  active_summary: string | null;
  jobs: MeetingJob[];
  included_in_search: boolean;
  deleted: boolean;
  recovery_notice?: string | null;
  recovery_copies?: Meeting[];
  reconcile_projection?: boolean;
  /** The recorded audio was deleted after a good transcript made it useless. */
  audio_removed: boolean;
  /** Why the audio was kept: 'empty_transcript' or 'interrupted_capture'; null while undecided or once removed. */
  audio_kept: string | null;
  /** Computed by the backend. List pages drop the chunks and transcripts, so rows read these two. */
  has_audio: boolean;
  has_text: boolean;
  /** Stopped with no server to transcribe with: it is transcribed when one is connected. */
  transcription_waiting?: boolean;
}
/** What the Meetings page needs from the settings and models to offer the next step. */
export interface MeetingReadiness {
  /** Why a transcription cannot start now (no model, no engine...); null when it can. */
  transcription_blocker: string | null;
  summary_ready: boolean;
}
export interface AudioCleanup {
  meetings: number;
  bytes: number;
}
export interface MeetingHit {
  meeting_id: number;
  title: string;
  kind: string;
  text: string;
  at_ms: number | null;
  version: string | null;
  segment_id: string | null;
}
export interface MeetingPage {
  items: MeetingDetail[];
  total: number;
  next_offset: number | null;
}
export function meetingMetadata(detail: MeetingDetail): MeetingDetail {
  return {
    ...detail,
    meeting: {
      ...detail.meeting,
      audio_path: null,
      transcript: '',
      notes: '',
      summary: '',
      key_points: [],
      action_items: [],
    },
    capture: { ...detail.capture, chunks: [], gaps: [], unverified_chunks: [] },
    transcripts: [],
    notes: [],
    note_history: [],
    summaries: [],
    legacy_transcript: null,
    legacy_notes: null,
    recovery_copies: [],
    reconcile_projection: false,
    jobs: detail.jobs.slice(-3),
  };
}
export interface SpeechModel {
  id: string;
  name: string;
  bytes: number;
  installed: boolean;
  license: string;
}
export interface TranscribeOptions {
  /** `compute` (the compute server) or `external` (the OpenAI-compatible endpoint). */
  provider: 'compute' | 'external';
  model: string;
  language: string;
}

let pendingMeetingOpen: MeetingHit | null = null;
const meetingOpenListeners = new Set<() => void>();
export function subscribeMeetingOpen(listener: () => void) {
  meetingOpenListeners.add(listener);
  return () => {
    meetingOpenListeners.delete(listener);
  };
}
export function requestMeetingOpen(hit: MeetingHit) {
  pendingMeetingOpen = structuredClone(hit);
  for (const listener of meetingOpenListeners) listener();
}
export function takeMeetingOpen(): MeetingHit | null {
  const hit = pendingMeetingOpen;
  pendingMeetingOpen = null;
  return hit;
}

export const meetingPreview = !isTauri();
export const captureActive = (capture: CaptureSnapshot) =>
  ['preparing', 'recording', 'paused', 'saving'].includes(capture.state);
export function clockTime(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(seconds / 3600);
  return `${hours ? `${hours}:` : ''}${String(Math.floor(seconds / 60) % 60).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
}
export const stateLabel = (state: string): string =>
  ({
    idle: tr('Audio non disponibile', 'Audio unavailable'),
    preparing: tr('Preparazione', 'Preparing'),
    recording: tr('Registrazione in corso', 'Recording'),
    paused: tr('In pausa', 'Paused'),
    saving: tr('Salvataggio audio', 'Saving audio'),
    saved: tr('Audio salvato', 'Audio saved'),
    interrupted: tr('Registrazione interrotta', 'Recording interrupted'),
    queued: tr('In attesa', 'Queued'),
    running: tr('Elaborazione in corso', 'Processing'),
    completed: tr('Completato', 'Completed'),
    failed: tr('Elaborazione non riuscita', 'Processing failed'),
    cancelled: tr('Annullato', 'Cancelled'),
  })[state] || state;

export class MeetingError extends Error {
  code: string;
  constructor(error: unknown) {
    const value =
      error && typeof error === 'object' ? (error as { message?: string; code?: string }) : {};
    super(value.message || String(error));
    this.code = value.code || 'external';
  }
}
// Preview only (test hook `meeting_preview_delay`): how long every answer is held back, as a real
// round trip to the backend holds it. The command has already run; only the reply is late.
let previewDelayMs = 0;
export async function meetingCall<T>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  try {
    if (!meetingPreview) return (await invoke(command, args)) as T;
    const answer = await previewMeeting(command, args);
    if (previewDelayMs > 0 && !command.startsWith('meeting_preview_'))
      await new Promise((resolve) => setTimeout(resolve, previewDelayMs));
    return answer as T;
  } catch (error) {
    throw error instanceof MeetingError ? error : new MeetingError(error);
  }
}

// Keeps edits available across navigation; flushing never discards a conflicting draft.
interface NoteDraft {
  text: string;
  revision: number;
  generation: number;
}
const noteDrafts = new Map<number, NoteDraft>();
const saves = new Map<number, Promise<MeetingDetail>>();
const deletions = new Map<number, Promise<void>>();
export const cachedMeetingDraft = (id: number) => noteDrafts.get(id);
export function cacheMeetingDraft(id: number, text: string, revision: number) {
  const old = noteDrafts.get(id);
  noteDrafts.set(id, {
    text,
    revision: old?.revision ?? revision,
    generation: (old?.generation ?? 0) + 1,
  });
}
export function resetMeetingDraft(id: number) {
  noteDrafts.delete(id);
}
export async function saveMeetingDraft(id: number): Promise<MeetingDetail | null> {
  const deleting = deletions.get(id);
  if (deleting) {
    await deleting;
    return null;
  }
  const inFlight = saves.get(id);
  if (inFlight) {
    await inFlight;
    return saveMeetingDraft(id);
  }
  const draft = noteDrafts.get(id);
  if (!draft) return null;
  const save = meetingCall<MeetingDetail>('meeting_save_notes', {
    id,
    text: draft.text,
    expectedRevision: draft.revision,
  });
  saves.set(id, save);
  try {
    const result = await save;
    const latest = noteDrafts.get(id);
    if (latest?.generation === draft.generation) noteDrafts.delete(id);
    else if (latest) latest.revision = result.revision;
    return result;
  } finally {
    saves.delete(id);
  }
}
// Deletion waits for a save already sent to the backend. Later flushes join the
// deletion instead of writing notes after it; failed deletion keeps the draft.
export function deleteMeeting(id: number): Promise<void> {
  const existing = deletions.get(id);
  if (existing) return existing;
  const pendingSave = saves.get(id);
  const deletion = (async () => {
    if (pendingSave) {
      try { await pendingSave; } catch { /* An unsaved draft does not prevent explicit deletion. */ }
    }
    await meetingCall('meeting_delete', { id });
    resetMeetingDraft(id);
  })();
  deletions.set(id, deletion);
  void deletion.finally(() => deletions.delete(id)).catch(() => {});
  return deletion;
}
export function rebaseMeetingDraft(id: number, revision: number) {
  const draft = noteDrafts.get(id);
  if (draft) draft.revision = revision;
}
export async function flushMeetingDrafts(): Promise<boolean> {
  try {
    await Promise.all([...deletions.values()]);
    for (const id of [...noteDrafts.keys()]) {
      await saveMeetingDraft(id);
      if (noteDrafts.has(id)) await saveMeetingDraft(id);
    }
    return noteDrafts.size === 0;
  } catch {
    return false;
  }
}

// Browser preview: synthetic data only. No microphone, filesystem or cloud access.
const now = () => Math.floor(Date.now() / 1000);
let nextId = 900000;
const sampleMeetings = (fixture as unknown as { meetings?: Meeting[] }).meetings || [];
const sampleDetail = (meeting: Meeting): MeetingDetail => ({
  schema_version: 2,
  revision: 1,
  meeting: structuredClone(meeting),
  request_id: '',
  capture: {
    ...emptyCapture(),
    meeting_id: meeting.id,
    state: 'saved',
    elapsed_ms: meeting.duration_secs * 1000,
    saved_ms: meeting.duration_secs * 1000,
    chunks: [
      {
        source: 'mixed',
        sequence: 0,
        path: 'synthetic.wav',
        start_ms: 0,
        end_ms: meeting.duration_secs * 1000,
        sample_rate: 16000,
        channels: 1,
      },
    ],
  },
  transcripts: [],
  active_transcript: null,
  notes: [],
  summaries: [],
  active_summary: null,
  jobs: [],
  included_in_search: true,
  deleted: false,
  audio_removed: false,
  audio_kept: null,
  has_audio: true,
  has_text: Boolean(meeting.transcript?.trim()),
});
// The sample data has one meeting, from before the transcript versions (its text is plain, which
// the one-time audio cleanup does not take). This second one is made like the service saves a
// meeting now: a complete transcript version with text, one clean audio chunk, no gaps. It is the
// one that cleanup finds, and the page lists it next to the old one.
const sampleTranscribed = (): MeetingDetail => {
  const first = sampleMeetings[0];
  const text = 'Sam: The pricing page copy is approved, we publish it on Monday.';
  const detail = sampleDetail({
    id: (first?.id ?? 19) - 1,
    title: 'Pricing review',
    date: (first?.date ?? 1793538498) - 3 * 86_400,
    duration_secs: 900,
    audio_path: null,
    transcript: text,
    notes: '',
    summary: '',
    key_points: [],
    action_items: [],
    event_id: null,
  });
  detail.transcripts = [
    {
      id: 'sample-transcript',
      created_at: detail.meeting.date,
      model: 'base',
      language: 'en',
      complete: true,
      segments: [
        {
          id: 'segment-1',
          start_ms: 0,
          end_ms: 900_000,
          text,
          original_text: text,
          source: 'mixed',
          confidence: null,
        },
      ],
    },
  ];
  detail.active_transcript = 'sample-transcript';
  return detail;
};
const demoMeetings: MeetingDetail[] = [...sampleMeetings.map(sampleDetail), sampleTranscribed()];
let demoCapture = emptyCapture();
let demoStarted = 0;
let demoElapsed = 0;
const demoModels: SpeechModel[] = [
  {
    id: 'base',
    name: 'Whisper Base · multilingue',
    bytes: 147951465,
    installed: false,
    license: 'MIT',
  },
];
const clone = <T>(value: T): T => structuredClone(value);
function getDemo(id: unknown): MeetingDetail {
  const found = demoMeetings.find((item) => item.meeting.id === Number(id) && !item.deleted);
  if (!found)
    throw { code: 'not_found', message: tr('Meeting non trovato.', 'Meeting not found.') };
  return found;
}
function checkRevision(detail: MeetingDetail, revision: unknown) {
  if (revision !== undefined && Number(revision) !== detail.revision)
    throw {
      code: 'revision_conflict',
      message: tr(
        'Gli appunti sono cambiati altrove. La tua bozza è conservata.',
        'These notes changed elsewhere. Your draft is safe.',
      ),
    };
}
function updateDemoCapture() {
  if (demoCapture.state === 'recording') {
    demoCapture.elapsed_ms = demoElapsed + Date.now() - demoStarted;
    demoCapture.saved_ms = Math.floor(demoCapture.elapsed_ms / 5000) * 5000;
    demoCapture.microphone_level = 0.16;
    demoCapture.system_level = 0.28;
  }
  if (demoCapture.meeting_id !== null) {
    const d = getDemo(demoCapture.meeting_id);
    d.capture = clone(demoCapture);
    d.meeting.duration_secs = Math.floor(demoCapture.elapsed_ms / 1000);
  }
}
function makeDemo(title: string): MeetingDetail {
  const id = nextId++;
  const detail: MeetingDetail = {
    schema_version: 2,
    revision: 1,
    meeting: {
      id,
      title,
      date: now(),
      duration_secs: 0,
      audio_path: null,
      transcript: '',
      notes: '',
      summary: '',
      key_points: [],
      action_items: [],
      event_id: null,
    },
    request_id: '',
    capture: emptyCapture(),
    transcripts: [],
    active_transcript: null,
    notes: [],
    summaries: [],
    active_summary: null,
    jobs: [],
    included_in_search: true,
    deleted: false,
    audio_removed: false,
    audio_kept: null,
    has_audio: false,
    has_text: false,
  };
  demoMeetings.unshift(detail);
  return detail;
}
// What a finished transcription / summary leaves in the meeting. `silent` is a recording with
// nothing in it: the transcript is complete and has no segment. `long` is a transcript of
// LONG_SEGMENTS lines, one every LONG_STEP_MS, for the tests that scroll to one of them. The
// version carries the model and the language the job was started with.
const LONG_SEGMENTS = 40;
const LONG_STEP_MS = 20_000;
function commitDemoTranscript(
  detail: MeetingDetail,
  silent = false,
  long = false,
  options: TranscribeOptions | null = null,
) {
  const id = crypto.randomUUID();
  const duration = Math.max(1000, detail.capture.elapsed_ms);
  const text = 'Confermiamo il prototipo per venerdì. Maria prepara la revisione del progetto.';
  const segment = (index: number, line: string): TranscriptSegment => ({
    id: `segment-${index + 1}`,
    start_ms: long ? index * LONG_STEP_MS : 0,
    end_ms: long ? (index + 1) * LONG_STEP_MS : duration,
    text: line,
    original_text: line,
    source: 'mixed',
    confidence: null,
  });
  const segments = silent
    ? []
    : long
      ? Array.from({ length: LONG_SEGMENTS }, (_, index) =>
          segment(
            index,
            `Passiamo all'argomento ${index + 1}: cosa serve per chiuderlo e chi se ne occupa.`,
          ),
        )
      : [segment(0, text)];
  detail.transcripts.push({
    id,
    created_at: now(),
    model: options?.model ?? 'base',
    language: options?.language ?? 'it',
    complete: true,
    segments,
  });
  detail.active_transcript = id;
  detail.meeting.transcript = segments.map((item) => item.text).join('\n');
}
function commitDemoSummary(detail: MeetingDetail) {
  const transcript = detail.transcripts.find((t) => t.id === detail.active_transcript);
  const sources: Evidence[] = transcript?.segments[0]
    ? [{ kind: 'segment', version: transcript.id, id: transcript.segments[0].id }]
    : [];
  const freeNote = detail.notes.find((note) => note.id === 'free' && note.text.trim());
  if (freeNote) sources.push({ kind: 'note', version: String(freeNote.revision), id: freeNote.id });
  const id = crypto.randomUUID();
  const summary: SummaryVersion = {
    id,
    created_at: now(),
    transcript_version: detail.active_transcript,
    input_revision: detail.revision,
    language: 'it',
    model: 'Demo sintetica',
    summary: 'Il team conferma il prototipo e prepara la revisione del progetto.',
    key_points: ['Prototipo previsto per venerdì.'],
    decisions: sources.length ? [{ text: 'Confermare il prototipo per venerdì.', sources }] : [],
    actions: sources.length
      ? [
          {
            id: 'action-1',
            text: 'Preparare la revisione del progetto.',
            owner: 'Maria',
            due: null,
            done: false,
            sources,
          },
        ]
      : [],
    complete: true,
  };
  detail.summaries.push(summary);
  detail.active_summary = id;
  detail.meeting.summary = summary.summary;
  detail.meeting.key_points = summary.key_points;
  detail.meeting.action_items = summary.actions.map((a) => a.text);
}

// The automatic chain of the simple Meetings page, as the real backend runs it: stop (or recover)
// queues `transcribe` when nothing blocks it; a transcript with text queues `summarize` when an
// assistant is set up; a good transcript of a clean recording deletes the audio.
// Progress is a function of the clock: each job remembers when it started and the commands that
// read a meeting move it forward (settleDemoRuns). No timer runs while nobody is asking.
const RUN_QUEUE_MS = 250;
const RUN_MS = { transcribe: 3000, summarize: 1500 } as const;
const RUN_STEPS = { transcribe: 4, summarize: 2 } as const;
type RunKind = keyof typeof RUN_MS;
interface DemoRun {
  meetingId: number;
  jobId: string;
  kind: RunKind;
  startedAt: number;
  silent: boolean;
  long: boolean;
  /** What a transcription runs with (the same as its job's); null for a summary. */
  options: TranscribeOptions | null;
  /** Why the job will end `failed`; null for a job that completes. */
  failure: string | null;
}
const demoRuns: DemoRun[] = [];
// What Settings says about the engine: the options every new transcription starts with. A retry
// of a job does not read it again, it runs with the options stored in the job (the real service
// does the same), so a fix made in Settings only reaches a new transcription. Test hook
// `meeting_preview_transcribe_options` changes it.
let demoTranscribeOptions: TranscribeOptions = { provider: 'compute', model: 'base', language: 'it' };
// Test hook `meeting_preview_chain_delay`: the real service queues the summary a moment after the
// transcription ends (once the worker has let go), so for a short while a meeting has a transcript
// and no job. 0 queues it in the same instant, as the preview always did.
let chainDelayMs = 0;
const demoSummaryQueue: { meetingId: number; at: number }[] = [];
let demoReadiness: MeetingReadiness = { transcription_blocker: null, summary_ready: true };
let silentNextTranscript = false;
let longNextTranscript = false;
const activeJob = (detail: MeetingDetail) =>
  detail.jobs.some((job) => ['queued', 'running'].includes(job.state));
function startDemoRun(
  detail: MeetingDetail,
  kind: RunKind,
  startedAt = Date.now(),
  stored: TranscribeOptions | null = null,
) {
  if (activeJob(detail))
    throw {
      code: 'job_running',
      message: tr(
        "Un'elaborazione è già in corso per questo meeting.",
        'A job is already running for this meeting.',
      ),
    };
  const job: MeetingJob = {
    id: crypto.randomUUID(),
    kind,
    state: 'queued',
    input_revision: detail.revision,
    progress: 0,
    error: null,
    created_at: Math.floor(startedAt / 1000),
  };
  // A transcription keeps the options it started with: the Settings of now, or the stored ones of
  // the job it retries.
  const options = kind === 'transcribe' ? clone(stored ?? demoTranscribeOptions) : null;
  if (options) {
    job.options = options;
    job.language = options.language;
  }
  detail.jobs.push(job);
  const silent = kind === 'transcribe' && silentNextTranscript;
  const long = kind === 'transcribe' && longNextTranscript;
  if (kind === 'transcribe') {
    silentNextTranscript = false;
    longNextTranscript = false;
  }
  // Like the real job: it is queued whatever the settings, and it is the run that finds the
  // assistant missing and ends `failed` with the reason.
  const failure =
    kind === 'summarize' && !demoReadiness.summary_ready
      ? tr(
          'Configura un assistente in Impostazioni per avere la sintesi.',
          'Set up an assistant in Settings to get the summary.',
        )
      : null;
  demoRuns.push({
    meetingId: detail.meeting.id,
    jobId: job.id,
    kind,
    startedAt,
    silent,
    long,
    options: options ? clone(options) : null,
    failure,
  });
}
// Like the service's `auto_transcribe`: nothing is queued while the engine is blocked, and a
// refusal (a job already running) is not an error of the command that asked.
// A meeting that already has a transcript, text, or a transcription job (even a failed one) is left
// alone: the person asks again from the page.
function autoTranscribeDemo(detail: MeetingDetail) {
  refreshDemoFlags(detail);
  if (
    detail.active_transcript ||
    detail.has_text ||
    detail.jobs.some((job) => job.kind === 'transcribe')
  )
    return;
  if (demoReadiness.transcription_blocker) return;
  try {
    startDemoRun(detail, 'transcribe');
  } catch {
    /* A job is already running: it will do. */
  }
}
function settleDemoRuns(at = Date.now()) {
  let moved = true;
  while (moved) {
    moved = false;
    for (const run of [...demoRuns]) {
      const detail = demoMeetings.find((item) => item.meeting.id === run.meetingId);
      const job = detail?.jobs.find((item) => item.id === run.jobId);
      if (!detail || !job || detail.deleted || !['queued', 'running'].includes(job.state)) {
        // Deleted, or cancelled by hand: the chain ends here.
        demoRuns.splice(demoRuns.indexOf(run), 1);
        continue;
      }
      const elapsed = at - run.startedAt;
      if (elapsed < RUN_MS[run.kind]) {
        const running = elapsed >= RUN_QUEUE_MS;
        job.state = running ? 'running' : 'queued';
        const share = running ? (elapsed - RUN_QUEUE_MS) / (RUN_MS[run.kind] - RUN_QUEUE_MS) : 0;
        job.progress = Math.floor(share * RUN_STEPS[run.kind]) * (100 / RUN_STEPS[run.kind]);
        continue;
      }
      demoRuns.splice(demoRuns.indexOf(run), 1);
      finishDemoRun(detail, job, run);
      moved = true;
    }
    for (const item of [...demoSummaryQueue]) {
      if (item.at > at) continue;
      demoSummaryQueue.splice(demoSummaryQueue.indexOf(item), 1);
      const detail = demoMeetings.find((entry) => entry.meeting.id === item.meetingId);
      if (!detail || detail.deleted) continue;
      try {
        startDemoRun(detail, 'summarize', item.at);
        moved = true;
      } catch {
        /* Another job got in first. */
      }
    }
  }
}
function finishDemoRun(detail: MeetingDetail, job: MeetingJob, run: DemoRun) {
  const endedAt = run.startedAt + RUN_MS[run.kind];
  if (run.failure) {
    job.state = 'failed';
    job.error = run.failure;
    return;
  }
  job.state = 'completed';
  job.progress = 100;
  if (run.kind === 'summarize') {
    commitDemoSummary(detail);
    detail.revision++;
    return;
  }
  commitDemoTranscript(detail, run.silent, run.long, run.options);
  detail.revision++;
  settleDemoAudio(detail);
  refreshDemoFlags(detail);
  if (detail.has_text && demoReadiness.summary_ready) {
    if (chainDelayMs > 0)
      demoSummaryQueue.push({ meetingId: detail.meeting.id, at: endedAt + chainDelayMs });
    else {
      try {
        startDemoRun(detail, 'summarize', endedAt);
      } catch {
        /* Another job got in first. */
      }
    }
  }
}
// The service's `refresh_flags`: what a list row reads in place of the chunks and the texts.
function refreshDemoFlags(detail: MeetingDetail) {
  detail.has_audio =
    !detail.audio_removed &&
    (detail.capture.chunks.length > 0 || Boolean(detail.meeting.audio_path));
  const transcript = detail.transcripts.find((item) => item.id === detail.active_transcript);
  detail.has_text =
    Boolean(transcript?.segments.some((segment) => segment.text.trim())) ||
    Boolean(detail.meeting.transcript.trim()) ||
    Boolean(detail.legacy_transcript?.trim());
}
// The service's `audio_verdict`: what to do with the recorded audio of a meeting. It needs a
// complete transcript VERSION: the plain text of a meeting from before the versions (the Weekly
// sync of the sample data) does not count, so its audio is never taken. A block that is missing or
// was never checked makes the capture "not clean" too.
function demoAudioVerdict(
  detail: MeetingDetail,
): 'pending' | 'remove' | 'empty_transcript' | 'interrupted_capture' {
  if (
    detail.deleted ||
    detail.audio_removed ||
    detail.capture.chunks.length === 0 ||
    detail.capture.state !== 'saved'
  )
    return 'pending';
  const transcript = detail.transcripts.find((item) => item.id === detail.active_transcript);
  if (!transcript?.complete) return 'pending';
  if (!transcript.segments.some((segment) => segment.text.trim())) return 'empty_transcript';
  if (
    detail.capture.incomplete ||
    detail.capture.gaps?.length ||
    detail.capture.missing_chunks?.length ||
    detail.capture.unverified_chunks?.length
  )
    return 'interrupted_capture';
  return 'remove';
}
// 16 kHz, 16 bit, mono: 32 bytes per millisecond.
const demoAudioBytes = (detail: MeetingDetail) =>
  detail.capture.chunks.reduce(
    (sum, chunk) => sum + Math.max(0, chunk.end_ms - chunk.start_ms),
    0,
  ) * 32;
const demoAudioRemoved = () => ({
  code: 'audio_removed',
  message: tr(
    "L'audio di questa riunione è stato eliminato dopo la trascrizione.",
    'The audio of this meeting was deleted after it was transcribed.',
  ),
});
// What is left of a capture once its audio is gone: no chunk, and none of the complaints about
// chunks that no longer exist (gaps, missing or unverified blocks, warnings).
function clearCaptureAudio(capture: CaptureSnapshot) {
  capture.chunks = [];
  capture.missing_chunks = [];
  capture.unverified_chunks = [];
  capture.gaps = [];
  capture.warnings = [];
  capture.incomplete = false;
  capture.error = null;
}
// Removing the audio is not an edit of the meeting: `revision` stays, so a page that is open on it
// can still toggle an action or rename without a conflict.
function removeDemoAudio(detail: MeetingDetail) {
  detail.audio_removed = true;
  detail.audio_kept = null;
  clearCaptureAudio(detail.capture);
  // The live capture state is copied into its meeting on every command: clear the source too.
  if (demoCapture.meeting_id === detail.meeting.id) clearCaptureAudio(demoCapture);
  refreshDemoFlags(detail);
}
function settleDemoAudio(detail: MeetingDetail) {
  const verdict = demoAudioVerdict(detail);
  if (verdict === 'remove') removeDemoAudio(detail);
  else if (verdict !== 'pending') detail.audio_kept = verdict;
}
function demoAudioCleanup(apply: boolean): AudioCleanup {
  const total: AudioCleanup = { meetings: 0, bytes: 0 };
  for (const detail of demoMeetings) {
    // A meeting a job holds is left to the next sweep, as in the service.
    if (demoAudioVerdict(detail) !== 'remove' || (apply && activeJob(detail))) continue;
    total.meetings++;
    total.bytes += demoAudioBytes(detail);
    if (apply) removeDemoAudio(detail);
  }
  return total;
}
const unreadableDemoMeetings = new Set<number>();
async function previewMeeting(command: string, args: Record<string, unknown>): Promise<unknown> {
  updateDemoCapture();
  settleDemoRuns();
  for (const detail of demoMeetings) refreshDemoFlags(detail);
  switch (command) {
    case 'meeting_readiness':
      return clone(demoReadiness);
    // Test hook: change what `meeting_readiness` answers. A field left out stays as it is.
    case 'meeting_preview_readiness':
      if ('transcription_blocker' in args)
        demoReadiness.transcription_blocker =
          args.transcription_blocker == null ? null : String(args.transcription_blocker);
      if ('summary_ready' in args) demoReadiness.summary_ready = Boolean(args.summary_ready);
      return clone(demoReadiness);
    // Test hook: the next transcription finds no words, as for a muted recording.
    case 'meeting_preview_empty_transcript':
      silentNextTranscript = true;
      return;
    // Test hook: hold every answer back by `ms` milliseconds (0 puts it right), so a test can see
    // the page while a command is in flight.
    case 'meeting_preview_delay':
      previewDelayMs = Math.max(0, Number(args.ms) || 0);
      return;
    // Test hook: the summary is queued `ms` milliseconds after the transcript is complete (0 puts it right).
    case 'meeting_preview_chain_delay':
      chainDelayMs = Math.max(0, Number(args.ms) || 0);
      return;
    // Test hook: the engine, model and language Settings says now (a field left out stays as it is).
    // Only transcriptions started after it use them; a retry keeps the options of its job.
    case 'meeting_preview_transcribe_options':
      if (args.provider === 'compute' || args.provider === 'external')
        demoTranscribeOptions.provider = args.provider;
      if (typeof args.model === 'string') demoTranscribeOptions.model = args.model;
      if (typeof args.language === 'string') demoTranscribeOptions.language = args.language;
      return clone(demoTranscribeOptions);
    // Test hook: the next transcription has forty timed lines, enough to need scrolling.
    case 'meeting_preview_long_transcript':
      longNextTranscript = true;
      return;
    case 'meeting_audio_cleanup_preview':
      return demoAudioCleanup(false);
    case 'meeting_audio_cleanup':
      return demoAudioCleanup(true);
    case 'meeting_devices':
      return [
        {
          id: 'demo-mic',
          name: tr('Microfono di esempio', 'Sample microphone'),
          kind: 'microphone',
          is_default: true,
        },
        {
          id: 'demo-system',
          name: tr('Audio computer di esempio', 'Sample computer audio'),
          kind: 'system',
          is_default: true,
        },
      ];
    case 'meeting_list':
      return clone(demoMeetings.filter((m) => !m.deleted));
    case 'meeting_list_page': {
      const offset = Math.max(0, Number(args.offset) || 0);
      const limit = Math.min(100, Math.max(1, Number(args.limit) || 50));
      const items = demoMeetings.filter((detail) => {
        const date = new Date(detail.meeting.date * 1000);
        const day = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
        return (
          !detail.deleted &&
          (!args.fromDate || day >= String(args.fromDate)) &&
          (!args.toDate || day <= String(args.toDate)) &&
          (!args.state || detail.capture.state === args.state)
        );
      });
      return clone({
        items: items.slice(offset, offset + limit).map(meetingMetadata),
        total: items.length,
        next_offset: offset + limit < items.length ? offset + limit : null,
      });
    }
    case 'meeting_read':
      if (unreadableDemoMeetings.has(Number(args.id)))
        throw new Error(
          tr('File meeting non leggibile (simulazione).', 'Meeting file unreadable (simulated).'),
        );
      return clone(getDemo(args.id));
    case 'meeting_preview_read_error':
      if (args.enabled === false) unreadableDemoMeetings.delete(Number(args.id));
      else unreadableDemoMeetings.add(Number(args.id));
      return;
    case 'meeting_capture_status':
      return clone(demoCapture);
    case 'meeting_preview_restart':
      demoCapture = emptyCapture();
      return;
    case 'meeting_start': {
      const request = args.request as {
        request_id: string;
        title?: string;
        event_id?: number;
        microphone?: string;
        system?: string;
      };
      const existing = demoMeetings.find((m) => m.request_id === request.request_id);
      if (existing) return clone(existing);
      if (captureActive(demoCapture))
        throw {
          code: 'recording_busy',
          message: tr('È già in corso una registrazione.', 'A recording is already in progress.'),
        };
      if (!request.microphone && !request.system)
        throw {
          code: 'device_unavailable',
          message: tr('Scegli almeno una sorgente.', 'Choose at least one source.'),
        };
      const d = makeDemo(request.title?.trim() || tr('Riunione di esempio', 'Sample meeting'));
      d.request_id = request.request_id;
      d.meeting.event_id = request.event_id ?? null;
      demoCapture = { ...emptyCapture(), meeting_id: d.meeting.id, state: 'recording' };
      demoStarted = Date.now();
      demoElapsed = 0;
      updateDemoCapture();
      return clone(d);
    }
    case 'meeting_pause':
      demoElapsed = demoCapture.elapsed_ms;
      demoCapture.state = 'paused';
      demoCapture.microphone_level = 0;
      demoCapture.system_level = 0;
      updateDemoCapture();
      return clone(demoCapture);
    case 'meeting_resume':
      demoStarted = Date.now();
      demoCapture.state = 'recording';
      updateDemoCapture();
      return clone(demoCapture);
    case 'meeting_stop': {
      const wasActive = captureActive(demoCapture);
      demoCapture.state = 'saved';
      demoCapture.saved_ms = demoCapture.elapsed_ms;
      demoCapture.microphone_level = 0;
      demoCapture.system_level = 0;
      demoCapture.chunks = [
        {
          source: 'mixed',
          sequence: 0,
          path: 'synthetic.wav',
          start_ms: 0,
          end_ms: demoCapture.elapsed_ms,
          sample_rate: 16000,
          channels: 1,
        },
      ];
      updateDemoCapture();
      // Nothing more to click: the transcription starts by itself (unless the engine is blocked).
      if (wasActive && demoCapture.meeting_id !== null)
        autoTranscribeDemo(getDemo(demoCapture.meeting_id));
      return clone(demoCapture);
    }
    case 'meeting_import': {
      const d = makeDemo(tr('Audio di esempio importato', 'Imported sample audio'));
      d.meeting.duration_secs = 60;
      d.capture = {
        ...emptyCapture(),
        meeting_id: d.meeting.id,
        state: 'saved',
        elapsed_ms: 60000,
        saved_ms: 60000,
        chunks: [
          {
            source: 'import',
            sequence: 0,
            path: 'synthetic.wav',
            start_ms: 0,
            end_ms: 60000,
            sample_rate: 16000,
            channels: 1,
          },
        ],
      };
      refreshDemoFlags(d);
      return clone(d);
    }
    case 'meeting_models':
      return clone(demoModels);
    // Like the service's `search_details`: every word of the query (split on spaces, any case) must
    // be in the same text, and the texts are the title, the lines of the transcript (or the plain
    // text of a meeting from before the versions), the notes and the summary.
    case 'meeting_search': {
      const terms = String(args.query || '')
        .toLocaleLowerCase()
        .split(/\s+/)
        .filter(Boolean);
      if (!terms.length) return [];
      return demoMeetings
        .filter((d) => !d.deleted && d.included_in_search)
        .flatMap((d) => {
          const hits: MeetingHit[] = [];
          const add = (
            kind: string,
            text: string,
            at_ms: number | null = null,
            version: string | null = null,
            segment_id: string | null = null,
          ) => {
            const lower = text.toLocaleLowerCase();
            if (terms.every((term) => lower.includes(term)))
              hits.push({ meeting_id: d.meeting.id, title: d.meeting.title, kind, text, at_ms, version, segment_id });
          };
          add('title', d.meeting.title);
          const transcript = d.transcripts.find((t) => t.id === d.active_transcript);
          if (transcript)
            for (const segment of transcript.segments)
              add('transcript', segment.text, segment.start_ms, transcript.id, segment.id);
          else add('transcript', d.meeting.transcript);
          add('notes', d.meeting.notes);
          add('summary', d.meeting.summary);
          return hits;
        });
    }
    default:
      break;
  }
  const d = getDemo(args.id);
  if (
    [
      'meeting_save_notes',
      'meeting_update',
      'meeting_toggle_action',
    ].includes(command)
  )
    checkRevision(d, args.expectedRevision);
  switch (command) {
    // Test hook: the meeting has no name of its own (one made from a calendar event can be so).
    case 'meeting_preview_untitle':
      d.meeting.title = '';
      break;
    // Test hook: the active summary gets more actions, one per text, so several boxes can be pressed.
    case 'meeting_preview_more_actions': {
      const summary = d.summaries.find((s) => s.id === d.active_summary);
      for (const text of (args.texts as string[] | undefined) ?? [])
        summary?.actions.push({
          id: `action-${summary.actions.length + 1}`,
          text,
          owner: null,
          due: null,
          done: false,
          sources: summary.actions[0]?.sources ?? [],
        });
      break;
    }
    case 'meeting_preview_interrupt': {
      d.capture.state = 'interrupted';
      d.capture.error = tr(
        'Registrazione interrotta: un blocco audio non è disponibile.',
        'Recording interrupted: an audio block is unavailable.',
      );
      d.capture.incomplete = true;
      d.capture.gaps = [{ source: 'microphone', start_ms: 5000, end_ms: 10000 }];
      d.capture.requires_receipts = true;
      d.capture.unverified_chunks = [d.capture.chunks[0]?.path || 'preview.wav'];
      d.capture.warnings = [
        tr(
          'Audio incompleto: manca il passaggio da 00:20 a 00:30.',
          'Incomplete audio: the passage from 00:20 to 00:30 is missing.',
        ),
      ];
      d.capture.saved_ms = 50000;
      d.capture.elapsed_ms = 60000;
      d.capture.missing_chunks = [
        {
          source: 'mixed',
          sequence: 1,
          path: 'synthetic-missing.wav',
          start_ms: 20000,
          end_ms: 30000,
          sample_rate: 16000,
          channels: 1,
        },
      ];
      d.recovery_notice = tr(
        'Metadati recuperati dalla copia disponibile. Verifica gli appunti e le versioni conservate.',
        'Metadata recovered from the available copy. Check the notes and the saved versions.',
      );
      d.recovery_copies = [
        {
          ...structuredClone(d.meeting),
          notes: 'Appunti più recenti recuperati: confermare il budget.',
        },
      ];
      demoCapture = clone(d.capture);
      break;
    }
    case 'meeting_save_notes': {
      const previous = d.notes.find((note) => note.id === 'free');
      if (previous) {
        (d.note_history ||= []).push(structuredClone(previous));
        previous.text = String(args.text || '');
        previous.revision++;
      } else d.notes.push({ id: 'free', text: String(args.text || ''), at_ms: null, revision: 1 });
      d.meeting.notes = String(args.text || '');
      d.revision++;
      break;
    }
    // Like the service: the event is looked for only when the link changes, so a meeting whose
    // event was deleted can still be renamed (the same event id comes back unchanged).
    case 'meeting_update': {
      const eventId = args.eventId == null ? null : Number(args.eventId);
      if (eventId !== null && eventId !== d.meeting.event_id) {
        const events = await call<{ id: number }[]>('list_events');
        if (!events.some((event) => event.id === eventId))
          throw { code: 'not_found', message: 'Evento not found' };
      }
      d.meeting.title = String(args.title || d.meeting.title);
      d.meeting.event_id = eventId;
      d.included_in_search = Boolean(args.includedInSearch);
      d.revision++;
      break;
    }
    case 'meeting_delete':
      if (d.meeting.id === demoCapture.meeting_id && captureActive(demoCapture))
        throw new Error(
          tr('Termina la registrazione prima di eliminare.', 'Stop the recording before deleting.'),
        );
      d.deleted = true;
      if (d.meeting.id === demoCapture.meeting_id) demoCapture = emptyCapture();
      return;
    case 'meeting_recover':
      d.capture.state = 'saved';
      d.capture.error = null;
      if (demoCapture.meeting_id === d.meeting.id) demoCapture = clone(d.capture);
      d.revision++;
      autoTranscribeDemo(d);
      break;
    // The Meetings page: the saved engine and language, no options to pick.
    case 'meeting_transcribe_default':
      if (demoReadiness.transcription_blocker)
        throw { code: 'validation', message: demoReadiness.transcription_blocker };
      if (captureActive(d.capture))
        throw {
          code: 'recording_busy',
          message: tr(
            "Termina la registrazione prima di elaborare l'audio.",
            'Stop the recording before processing the audio.',
          ),
        };
      if (d.audio_removed) throw demoAudioRemoved();
      startDemoRun(d, 'transcribe');
      break;
    // Queued whatever the settings say; with no assistant the job ends `failed` and says why.
    case 'meeting_summarize_default':
      if (captureActive(d.capture))
        throw {
          code: 'recording_busy',
          message: tr(
            "Termina la registrazione prima di elaborare l'audio.",
            'Stop the recording before processing the audio.',
          ),
        };
      if (!d.active_transcript && !d.has_text && !d.meeting.notes.trim() && !d.notes.length)
        throw {
          code: 'validation',
          message: tr(
            'Aggiungi una trascrizione o degli appunti.',
            'Add a transcript or some notes.',
          ),
        };
      startDemoRun(d, 'summarize');
      break;
    // The manual way out for audio that was kept (empty transcript, recording with gaps).
    case 'meeting_discard_audio':
      if (!d.audio_kept || d.audio_removed)
        throw {
          code: 'validation',
          message: tr("Non c'è audio da eliminare.", 'There is no audio to delete.'),
        };
      if (activeJob(d))
        throw {
          code: 'job_running',
          message: tr(
            "Un'elaborazione è già in corso per questo meeting.",
            'A job is already running for this meeting.',
          ),
        };
      removeDemoAudio(d);
      break;
    case 'meeting_cancel_job': {
      const job = d.jobs.find((j) => j.id === args.jobId);
      if (job && ['running', 'queued'].includes(job.state)) job.state = 'cancelled';
      break;
    }
    // Runs a failed, interrupted or cancelled job again, as the service does: with what the job was
    // started with, not with what Settings says now (a fixed engine only helps a new transcription).
    case 'meeting_retry_job': {
      const job = d.jobs.find((j) => j.id === args.jobId);
      if (!job) throw { code: 'not_found', message: 'Lavoro not found' };
      if (!['failed', 'interrupted', 'cancelled'].includes(job.state))
        throw {
          code: 'validation',
          message: tr(
            'Questo lavoro non richiede un nuovo tentativo.',
            'This job does not need another try.',
          ),
        };
      if (job.kind !== 'transcribe' && job.kind !== 'summarize')
        throw {
          code: 'validation',
          message: tr(
            'Scegli nuovamente il file con Importa audio.',
            'Choose the file again with Import audio.',
          ),
        };
      if (job.kind === 'transcribe' && d.audio_removed) throw demoAudioRemoved();
      startDemoRun(d, job.kind, Date.now(), job.options ?? null);
      break;
    }
    case 'meeting_toggle_action': {
      const summary = d.summaries.find((s) => s.id === d.active_summary);
      const action = summary?.actions.find((a) => a.id === args.actionId);
      if (action) action.done = Boolean(args.done);
      d.revision++;
      break;
    }
    default:
      throw new Error(
        tr(
          `Comando meeting non disponibile: ${command}`,
          `Meeting command not available: ${command}`,
        ),
      );
  }
  refreshDemoFlags(d);
  return clone(d);
}
