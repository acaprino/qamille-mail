// The logic behind the simple Meetings page, with no screen in it: it turns meetings and calendar
// events into list rows and a meeting into the state its detail is in. It returns codes and data;
// the components write the words. Pure TypeScript on purpose (no Svelte, no i18n, no Tauri, no
// clock): the time of day always comes in as `now`, in unix seconds.
import type {
  AudioDevice,
  MeetingDetail,
  MeetingHit,
  MeetingJob,
  MeetingReadiness,
} from './meetings';
import type { CalendarEvent } from './types';

// ─── The list ───────────────────────────────────────────────────────────────────────────────────

export type MeetingGroup = 'live' | 'processing' | 'upcoming' | 'past';
export type RowStatus =
  | 'recording'
  | 'processing'
  | 'ready'
  | 'interrupted'
  | 'to_transcribe'
  | 'empty'
  | 'failed'
  | 'unrecorded'
  | 'upcoming';

export interface MeetingRow {
  /** `m:<meeting id>` for a recording, `e:<event id>` for an event nobody recorded. */
  key: string;
  group: MeetingGroup;
  status: RowStatus;
  meetingId: number | null;
  eventId: number | null;
  title: string;
  /** Unix seconds: the meeting date, or the start of the event. */
  start: number;
  durationSecs: number | null;
  attendees: number | null;
  place: string | null;
  meetingLink: string | null;
  /** Only for an event in Prossime that has already begun. */
  startedSecsAgo: number | null;
  /** The most recent transcribe/summarize job, for a row in elaborazione. `progress` is 0-100. */
  job: { kind: 'transcribe' | 'summarize'; progress: number } | null;
  /** False for an event without a recording: there is nothing to open. */
  openable: boolean;
}

export const UPCOMING_DAYS = 7;
export const PAST_DAYS = 14;
const DAY_SECS = 86_400;
/** An event this long or longer is an all-day one. 23 h, not 24: on a spring-forward date a day lasts 23 h. */
const ALL_DAY_SECS = 23 * 3600;

/** The groups of the list, top to bottom. */
export const GROUP_ORDER: MeetingGroup[] = ['live', 'processing', 'upcoming', 'past'];
const LIVE_CAPTURE = ['preparing', 'recording', 'paused', 'saving'];
const ACTIVE_JOB = ['queued', 'running'];
const FAILED_JOB = ['failed', 'cancelled', 'interrupted'];

const isLive = (detail: MeetingDetail) => LIVE_CAPTURE.includes(detail.capture.state);
const isActive = (job: MeetingJob) => ACTIVE_JOB.includes(job.state);
const isFailed = (job: MeetingJob) => FAILED_JOB.includes(job.state);

/** True while a transcription or a summary of the meeting waits or runs. */
export function hasActiveJob(detail: MeetingDetail): boolean {
  return detail.jobs.some(
    (job) => (job.kind === 'transcribe' || job.kind === 'summarize') && isActive(job),
  );
}

/**
 * The meetings the page must keep reading while something happens to them: the ones in
 * elaborazione (not being recorded, a transcription or a summary waiting or running). Ascending,
 * each once (the open meeting may be in the list as well as on its own).
 */
export function processingIds(details: MeetingDetail[]): number[] {
  const ids = new Set<number>();
  for (const detail of details)
    if (!detail.deleted && !isLive(detail) && hasActiveJob(detail)) ids.add(detail.meeting.id);
  return [...ids].sort((a, b) => a - b);
}

/**
 * True when the transcription of the meeting has finished and no summary job came after it. The
 * service queues the summary a moment after the transcription ends (once its worker let go), so
 * this is what a meeting looks like in that moment, and also what it looks like when no summary
 * will come (no assistant): the page cannot tell which, and waits a little.
 */
export function awaitsSummary(detail: MeetingDetail): boolean {
  if (hasActiveJob(detail)) return false;
  let at = -1;
  detail.jobs.forEach((job, index) => {
    if (job.kind === 'transcribe') at = index;
  });
  return (
    at !== -1 &&
    detail.jobs[at].state === 'completed' &&
    !detail.jobs.slice(at + 1).some((job) => job.kind === 'summarize')
  );
}

/**
 * What the page watches of the recording: whether one is going on and which meeting it is. The
 * list is read again when this changes (a recording started or stopped elsewhere), never while it
 * stays the same (paused, resumed, the clock moving on).
 */
export function captureMark(capture: { state: string; meeting_id: number | null }): string {
  return `${LIVE_CAPTURE.includes(capture.state) ? 'on' : 'off'}:${capture.meeting_id ?? ''}`;
}

/**
 * The meeting whose recording just ended between two snapshots, if one did: it was being recorded
 * and now nothing is, or another meeting is. Its transcription is queued a moment after the stop.
 */
export function stoppedMeeting(
  before: { state: string; meeting_id: number | null },
  after: { state: string; meeting_id: number | null },
): number | null {
  if (before.meeting_id === null || !LIVE_CAPTURE.includes(before.state)) return null;
  const same = LIVE_CAPTURE.includes(after.state) && after.meeting_id === before.meeting_id;
  return same ? null : before.meeting_id;
}

/** The newest job that passes `match`; among equal dates, the one added last. */
function latestJob(jobs: MeetingJob[], match: (job: MeetingJob) => boolean): MeetingJob | null {
  let best: MeetingJob | null = null;
  for (const job of jobs)
    if (match(job) && (!best || job.created_at >= best.created_at)) best = job;
  return best;
}

function meetingRow(detail: MeetingDetail, event: CalendarEvent | undefined): MeetingRow {
  const active = latestJob(
    detail.jobs,
    (job) => (job.kind === 'transcribe' || job.kind === 'summarize') && isActive(job),
  );
  const lastTranscribe = latestJob(detail.jobs, (job) => job.kind === 'transcribe');
  let group: MeetingGroup = 'past';
  let status: RowStatus = 'ready';
  if (isLive(detail)) {
    group = 'live';
    status = 'recording';
  } else if (active) {
    group = 'processing';
    status = 'processing';
  } else if (detail.capture.state === 'interrupted') {
    status = 'interrupted';
  } else if (!detail.has_text && lastTranscribe && isFailed(lastTranscribe)) {
    status = 'failed';
  } else if (
    detail.capture.state === 'saved' &&
    !detail.has_text &&
    detail.audio_kept === 'empty_transcript' &&
    !detail.audio_removed
  ) {
    status = 'empty';
  } else if (detail.capture.state === 'saved' && detail.has_audio && !detail.has_text) {
    status = 'to_transcribe';
  }
  const m = detail.meeting;
  return {
    key: `m:${m.id}`,
    group,
    status,
    meetingId: m.id,
    eventId: m.event_id,
    title: m.title.trim() || event?.title || m.title,
    start: m.date,
    durationSecs: m.duration_secs > 0 ? m.duration_secs : null,
    attendees: event ? event.attendees.length || null : null,
    place: event?.location.trim() || null,
    meetingLink: event?.meeting_link || null,
    startedSecsAgo: null,
    job:
      group === 'processing' && active
        ? { kind: active.kind as 'transcribe' | 'summarize', progress: active.progress }
        : null,
    openable: true,
  };
}

function eventRow(event: CalendarEvent, group: 'upcoming' | 'past', now: number): MeetingRow {
  return {
    key: `e:${event.id}`,
    group,
    status: group === 'upcoming' ? 'upcoming' : 'unrecorded',
    meetingId: null,
    eventId: event.id,
    title: event.title,
    start: event.start,
    durationSecs: Math.max(0, event.end - event.start) || null,
    attendees: event.attendees.length || null,
    place: event.location.trim() || null,
    meetingLink: event.meeting_link || null,
    startedSecsAgo: group === 'upcoming' && event.start < now ? now - event.start : null,
    job: null,
    openable: false,
  };
}

/**
 * The rows of the page, in the order they are shown. A recording that belongs to a calendar event
 * takes the place of the event; events nobody recorded stay as grey rows that do not open. Events
 * that last a day or more (23 hours or more), end before they start, or lie outside the window (a
 * week ahead, two weeks back), are left out.
 */
export function mergeMeetingRows(input: {
  meetings: MeetingDetail[];
  events: CalendarEvent[];
  now: number;
}): MeetingRow[] {
  const { now } = input;
  const meetings = input.meetings.filter((detail) => !detail.deleted);
  const eventById = new Map(input.events.map((event) => [event.id, event]));
  const covered = new Set<number>();
  const rows: MeetingRow[] = [];
  for (const detail of meetings) {
    const eventId = detail.meeting.event_id;
    if (eventId !== null) covered.add(eventId);
    rows.push(meetingRow(detail, eventId === null ? undefined : eventById.get(eventId)));
  }
  const upcomingLimit = now + UPCOMING_DAYS * DAY_SECS;
  const pastLimit = now - PAST_DAYS * DAY_SECS;
  for (const event of input.events) {
    if (covered.has(event.id) || event.end < event.start || event.end - event.start >= ALL_DAY_SECS)
      continue;
    // An event the person declined is not offered to join and record.
    if (event.end > now && event.start < upcomingLimit) {
      if (event.rsvp !== 'Declined') rows.push(eventRow(event, 'upcoming', now));
    } else if (event.end <= now && event.start >= pastLimit) rows.push(eventRow(event, 'past', now));
  }
  const groups = groupRows(rows);
  return GROUP_ORDER.flatMap((group) => groups[group]);
}

/**
 * The next moment (unix seconds, always after `now`) at which a row of Prossime changes by itself,
 * or `null` when none will: the list is built from the clock, and a page at rest has to know when
 * to look at it again. For an event that has not begun it is the second after its start (the row
 * turns into "Iniziata da N min"); for one that has begun it is the next whole minute counted from
 * its start (the N moves) or its end, whichever comes first (the row leaves Prossime). Recordings
 * and past events do not depend on the clock.
 */
export function nextClockBoundary(rows: MeetingRow[], now: number): number | null {
  let next: number | null = null;
  const consider = (at: number) => {
    if (at > now && (next === null || at < next)) next = at;
  };
  for (const row of rows) {
    if (row.status !== 'upcoming') continue;
    consider(row.start + (row.durationSecs ?? 0));
    consider(
      row.start < now
        ? row.start + (Math.floor((now - row.start) / 60) + 1) * 60
        : Math.max(row.start, now) + 1,
    );
  }
  return next;
}

/** Rows by group: Prossime soonest first, every other group newest first. Does not change `rows`. */
export function groupRows(rows: MeetingRow[]): Record<MeetingGroup, MeetingRow[]> {
  const groups: Record<MeetingGroup, MeetingRow[]> = {
    live: [],
    processing: [],
    upcoming: [],
    past: [],
  };
  for (const row of rows) groups[row.group].push(row);
  for (const group of GROUP_ORDER) {
    const direction = group === 'upcoming' ? 1 : -1;
    groups[group].sort(
      (a, b) => direction * (a.start - b.start) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0),
    );
  }
  return groups;
}

// ─── The detail ─────────────────────────────────────────────────────────────────────────────────

export type AudioNote = null | 'removed' | 'empty_transcript' | 'interrupted_capture';
export type SummaryView =
  | { kind: 'text' }
  | { kind: 'ai_missing' }
  | { kind: 'failed'; message: string; jobId: string }
  | { kind: 'pending'; job: MeetingJob }
  | { kind: 'generate' };
export type DetailState =
  | { kind: 'live' }
  | { kind: 'processing'; job: MeetingJob }
  | { kind: 'recover' }
  | {
      kind: 'no_transcript';
      reason: 'blocked' | 'failed' | 'empty' | 'never';
      message: string | null;
      failedJob: MeetingJob | null;
    }
  | { kind: 'ready'; summary: SummaryView; audio: AudioNote };

const hasSummaryText = (detail: MeetingDetail) =>
  Boolean(
    (detail.active_summary && detail.summaries.some((s) => s.id === detail.active_summary)) ||
    detail.meeting.summary.trim(),
  );

function summaryView(detail: MeetingDetail, readiness: MeetingReadiness): SummaryView {
  if (hasSummaryText(detail)) return { kind: 'text' };
  const last = latestJob(detail.jobs, (job) => job.kind === 'summarize');
  if (last && isFailed(last)) return { kind: 'failed', message: last.error ?? '', jobId: last.id };
  const running = latestJob(detail.jobs, (job) => job.kind === 'summarize' && isActive(job));
  if (running) return { kind: 'pending', job: running };
  return readiness.summary_ready ? { kind: 'generate' } : { kind: 'ai_missing' };
}

function audioNote(detail: MeetingDetail): AudioNote {
  if (detail.audio_removed) return 'removed';
  if (detail.audio_kept === 'empty_transcript' || detail.audio_kept === 'interrupted_capture')
    return detail.audio_kept;
  return null;
}

/**
 * What the detail of a meeting shows, first match wins: recording now, working on it, recover an
 * interrupted capture, no transcript yet (and why), otherwise the finished meeting. A summary that
 * is being written does not hide a transcript that already exists: that is `ready` with a
 * `pending` summary (carrying the job, for its bar), so the page only waits for the missing part.
 * A transcript that came out empty keeps the audio and says so (`reason: 'empty'`); a newer failed
 * transcription still comes first, and an empty one comes before a blocker.
 */
export function detailState(
  detail: MeetingDetail,
  ctx: { readiness: MeetingReadiness },
): DetailState {
  const { readiness } = ctx;
  if (isLive(detail)) return { kind: 'live' };
  const working = latestJob(
    detail.jobs,
    (job) =>
      isActive(job) &&
      (job.kind === 'transcribe' || (job.kind === 'summarize' && !detail.has_text)),
  );
  if (working) return { kind: 'processing', job: working };
  if (detail.capture.state === 'interrupted') return { kind: 'recover' };
  if (!detail.has_text) {
    const last = latestJob(detail.jobs, (job) => job.kind === 'transcribe');
    if (last && isFailed(last))
      return { kind: 'no_transcript', reason: 'failed', message: last.error, failedJob: last };
    if (detail.audio_kept === 'empty_transcript' && !detail.audio_removed)
      return { kind: 'no_transcript', reason: 'empty', message: null, failedJob: null };
    if (readiness.transcription_blocker !== null)
      return {
        kind: 'no_transcript',
        reason: 'blocked',
        message: readiness.transcription_blocker,
        failedJob: null,
      };
    return { kind: 'no_transcript', reason: 'never', message: null, failedJob: null };
  }
  return { kind: 'ready', summary: summaryView(detail, readiness), audio: audioNote(detail) };
}

// ─── Transcript and summary text ───────────────────────────────────────────────────────────────

const two = (value: number) => String(value).padStart(2, '0');

/** `mm:ss`, and `hh:mm:ss` from one hour on. Anything negative or not a number is 00:00. */
export function clockLabel(ms: number): string {
  const total = Number.isFinite(ms) ? Math.max(0, Math.floor(ms / 1000)) : 0;
  const hours = Math.floor(total / 3600);
  const clock = `${two(Math.floor(total / 60) % 60)}:${two(total % 60)}`;
  return hours ? `${two(hours)}:${clock}` : clock;
}

/** The lines of the transcript. Without a transcript version, the plain text is one line with no time. */
export function transcriptRows(
  detail: MeetingDetail,
): { id: string; time: string; text: string }[] {
  const active = detail.active_transcript
    ? detail.transcripts.find((t) => t.id === detail.active_transcript)
    : undefined;
  const rows = (active?.segments ?? [])
    .filter((segment) => segment.text.trim())
    .map((segment) => ({ id: segment.id, time: clockLabel(segment.start_ms), text: segment.text }));
  if (rows.length) return rows;
  const plain = [detail.legacy_transcript, detail.meeting.transcript].find((text) => text?.trim());
  return plain ? [{ id: 'plain', time: '', text: plain }] : [];
}

/** The transcript as text to copy: one `mm:ss text` line per row. */
export function transcriptPlainText(detail: MeetingDetail): string {
  return transcriptRows(detail)
    .map((row) => (row.time ? `${row.time} ${row.text}` : row.text))
    .join('\n');
}

/**
 * The summary as text to copy: the summary, then key points, decisions and actions as bullet lists,
 * blocks separated by a blank line. A meeting from before the versions falls back to its own fields.
 */
export function summaryPlainText(detail: MeetingDetail): string {
  const active = detail.active_summary
    ? detail.summaries.find((s) => s.id === detail.active_summary)
    : undefined;
  const bullets = (lines: string[]) => lines.map((line) => `- ${line}`).join('\n');
  const blocks = active
    ? [
        active.summary.trim(),
        bullets(active.key_points),
        bullets(active.decisions.map((decision) => decision.text)),
        bullets(
          active.actions.map((action) => {
            const who = [action.owner, action.due].filter(Boolean).join(', ');
            return who ? `${action.text} (${who})` : action.text;
          }),
        ),
      ]
    : [
        detail.meeting.summary.trim(),
        bullets(detail.meeting.key_points),
        bullets(detail.meeting.action_items),
      ];
  return blocks.filter(Boolean).join('\n\n');
}

// ─── Search hits ────────────────────────────────────────────────────────────────────────────────

/** Hits the page can open. The page has no notes, so a hit in a note leads nowhere. */
export function usableHits(hits: MeetingHit[]): MeetingHit[] {
  return hits.filter((hit) => hit.kind !== 'notes' && hit.kind !== 'note');
}

/** A hit in the transcript opens it at that segment; every other hit opens the summary. */
export function hitTarget(hit: MeetingHit): {
  tab: 'summary' | 'transcript';
  segmentId: string | null;
} {
  return hit.kind === 'transcript'
    ? { tab: 'transcript', segmentId: hit.segment_id }
    : { tab: 'summary', segmentId: null };
}

// ─── Devices ────────────────────────────────────────────────────────────────────────────────────

/**
 * The microphone and the system audio to record with: the configured device if it is still there,
 * otherwise the default one of its kind. Without a default microphone, the first one; without a
 * default output, no system audio. Mirrors `recording_devices` in app/src/domain/reminders.rs.
 */
export function pickDevices(
  devices: AudioDevice[],
  settings: { meeting_microphone: string; meeting_system_audio: string },
): { microphone: string | null; system: string | null } {
  const microphones = devices.filter(isMicrophone);
  const outputs = devices.filter(isSystemAudio);
  const configured = (list: AudioDevice[], id: string) =>
    id ? (list.find((d) => d.id === id)?.id ?? null) : null;
  return {
    microphone:
      configured(microphones, settings.meeting_microphone) ??
      (microphones.find((d) => d.is_default) ?? microphones[0])?.id ??
      null,
    system:
      configured(outputs, settings.meeting_system_audio) ??
      outputs.find((d) => d.is_default)?.id ??
      null,
  };
}

/** The devices that capture a voice, and those that carry the computer's audio: Settings and the recording share the rule. */
export const isMicrophone = (device: { kind: string }): boolean =>
  device.kind === 'microphone' || device.kind === 'input';
export const isSystemAudio = (device: { kind: string }): boolean =>
  device.kind === 'system' || device.kind === 'output';
